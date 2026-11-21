//! Deletion engine for platforms without the `openat` family (Windows).
//!
//! Same contract as the unix engine (`remove_all`, `remove_all_interactive`,
//! see their docs in lib.rs), built from path-based `std::fs` calls and
//! `std::thread` only:
//!
//! * The calling thread walks each operand iteratively (an explicit stack, no
//!   recursion). Files go into batches of `BATCH_ENTRIES`; the first full
//!   batch starts a pool of `jobs` workers (0 = min(cores, 4)) that delete
//!   them while the walk goes on. Small jobs never leave the calling thread.
//! * Directories are removed once their operand has been walked and its files
//!   are gone, deepest level first. Directories on one level are independent,
//!   so a wide level is spread over the pool as well. A failure anywhere
//!   below a directory keeps it and every ancestor, without a spurious
//!   "Directory not empty" for each of them.
//! * Symlinks and junctions are never followed: entries are classified with
//!   `symlink_metadata` / `DirEntry::file_type`, which do not traverse them. A
//!   directory symlink or junction is removed with `remove_dir` (that removes
//!   the link, not its target), anything else that is not a directory with
//!   `remove_file`.
//!
//! Differences from the unix engine:
//!
//! * The unix engine never builds a path on the hot path and opens every
//!   directory with `O_NOFOLLOW`, so a directory swapped for a symlink during
//!   the run can never be followed. Stable std has no handle-relative calls,
//!   so here each directory is re-checked with `symlink_metadata` right before
//!   it is listed. That narrows the window to microseconds but cannot close it.
//! * A read-only *file* (FILE_ATTRIBUTE_READONLY) cannot be deleted on
//!   Windows, and a directory's own permissions never stop deleting its
//!   entries. So `force_writable` means: on "Permission denied", clear the
//!   read-only attribute of the entry itself and retry once. Without it the
//!   error is reported. Note that git marks the files under `.git/objects`
//!   read-only on Windows.
//! * `one_file_system` needs a volume id, which stable std does not expose on
//!   Windows, so every operand is refused with
//!   `--one-file-system is not supported on this platform` instead of
//!   silently crossing into other volumes.
//! * Byte totals count a hard-linked file once per link: stable std exposes
//!   neither the link count nor the file index on Windows.
//! * Windows can keep a deleted file's name around until the last handle to it
//!   closes (virus scanners, indexers), so an rmdir that fails with
//!   "Directory not empty" although nothing below failed is retried a few
//!   times with a short backoff before it is reported.

use std::fmt::Write as _;
use std::fs::{self, FileType};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::{DeleteStats, Options, Outcome};

/// Files per batch handed to a worker.
const BATCH_ENTRIES: usize = 128;

/// Directories per job when a level is spread over the pool.
const DIR_CHUNK: usize = 32;

/// Upper bound for the automatic worker count (deletes are metadata writes
/// and stop scaling long before the core count).
const MAX_AUTO_WORKERS: usize = 4;

/// Queue depth per worker: bounds memory and how far the walker runs ahead.
const QUEUE_PER_WORKER: usize = 2;

/// Retries for an rmdir that fails with "Directory not empty" although
/// nothing below failed. The sleeps double from 1ms (127ms in total).
const NOT_EMPTY_RETRIES: u32 = 7;

/// "No directory": the entry is an operand itself.
const NO_DIR: usize = usize::MAX;

const NO_ONE_FS: &str = "--one-file-system is not supported on this platform";

fn worker_count(jobs: usize) -> usize {
    if jobs > 0 {
        return jobs;
    }
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, MAX_AUTO_WORKERS)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn write_stdout(text: &str) {
    if text.is_empty() {
        return;
    }
    let _ = io::stdout().lock().write_all(text.as_bytes());
}

/// The reason part of an error message: the POSIX wording for the common
/// cases, so messages read the same on every platform, otherwise the OS text
/// without Rust's " (os error N)" suffix.
fn reason(e: &io::Error) -> String {
    use io::ErrorKind as K;
    let text = match e.kind() {
        K::NotFound => "No such file or directory",
        K::PermissionDenied => "Permission denied",
        K::DirectoryNotEmpty => "Directory not empty",
        K::IsADirectory => "Is a directory",
        K::NotADirectory => "Not a directory",
        K::ReadOnlyFilesystem => "Read-only file system",
        K::ResourceBusy => "Device or resource busy",
        _ => {
            let full = e.to_string();
            let text = match full.rfind(" (os error ") {
                Some(i) => &full[..i],
                None => &full,
            };
            return text.trim_end().trim_end_matches('.').to_string();
        }
    };
    text.to_string()
}

fn cannot_remove(path: &Path, why: &str) -> String {
    format!("cannot remove '{}': {why}", path.display())
}

/// How a non-directory entry is deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Files, file symlinks, anything else: `remove_file`.
    File,
    /// Directory symlinks and junctions (Windows): `remove_dir`, which removes
    /// the link itself and never touches its target. Unix has no such entries.
    #[cfg_attr(not(windows), allow(dead_code))]
    DirLink,
}

fn kind_of(ft: &FileType) -> Kind {
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTypeExt;
        if ft.is_symlink_dir() {
            return Kind::DirLink;
        }
    }
    let _ = ft;
    Kind::File
}

fn unlink(path: &Path, kind: Kind) -> io::Result<()> {
    let as_dir = kind == Kind::DirLink;
    let remove = |dir: bool| {
        if dir {
            fs::remove_dir(path)
        } else {
            fs::remove_file(path)
        }
    };
    match remove(as_dir) {
        // Whether a link counts as a file or a directory link depends on how
        // it was created and on the file system (wine, SMB shares): if the
        // entry is still a link, the other call is the right one. Neither
        // call follows the link, so this never touches the target.
        Err(e) if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) => {
            remove(!as_dir).map_err(|_| e)
        }
        r => r,
    }
}

/// Clear the read-only attribute of `path` itself (never of a link's target).
/// Returns whether it was set and is now cleared.
#[cfg(windows)]
#[allow(clippy::permissions_set_readonly_false)] // clears FILE_ATTRIBUTE_READONLY
fn clear_readonly(path: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    // set_permissions follows links.
    if meta.file_type().is_symlink() {
        return false;
    }
    let mut perm = meta.permissions();
    if !perm.readonly() {
        return false;
    }
    perm.set_readonly(false);
    fs::set_permissions(path, perm).is_ok()
}

/// Off Windows this module only runs as a test build on unix, where
/// "read-only" would mean the mode bits of the containing directory.
#[cfg(not(windows))]
fn clear_readonly(_path: &Path) -> bool {
    false
}

/// `force_writable` applies to this error: the attribute was cleared, retry.
fn retry_writable(e: &io::Error, path: &Path, opts: &Options) -> bool {
    opts.force_writable && e.kind() == io::ErrorKind::PermissionDenied && clear_readonly(path)
}

/// Delete a non-directory entry.
fn delete_entry(path: &Path, kind: Kind, opts: &Options) -> io::Result<()> {
    match unlink(path, kind) {
        Err(e) if retry_writable(&e, path, opts) => unlink(path, kind),
        r => r,
    }
}

/// Remove a directory that should be empty by now.
///
/// `Ok(true)`: removed (or, in a dry run, would be). `Ok(false)`: it was
/// already gone and `force` is set. `Err`: the message to report. When the
/// directory could not be listed earlier, that error (`open_err`) is the one
/// reported if the rmdir fails too, as the unix engine does.
fn remove_dir(path: &Path, open_err: Option<&str>, opts: &Options) -> Result<bool, String> {
    if opts.dry_run {
        // Listing failed: whether rmdir would work cannot be known.
        return match open_err {
            Some(why) => Err(cannot_remove(path, why)),
            None => Ok(true),
        };
    }
    let mut delay = Duration::from_millis(1);
    let mut retries = 0;
    let mut cleared = false;
    loop {
        let e = match fs::remove_dir(path) {
            Ok(()) => return Ok(true),
            Err(e) => e,
        };
        match e.kind() {
            io::ErrorKind::NotFound if opts.force => return Ok(false),
            io::ErrorKind::DirectoryNotEmpty
                if open_err.is_none() && retries < NOT_EMPTY_RETRIES =>
            {
                std::thread::sleep(delay);
                delay *= 2;
                retries += 1;
            }
            _ if !cleared && retry_writable(&e, path, opts) => cleared = true,
            _ => return Err(cannot_remove(path, open_err.unwrap_or(&reason(&e)))),
        }
    }
}

fn add_progress(opts: &Options, stats: &DeleteStats) {
    if let Some(p) = opts.progress {
        p.files
            .fetch_add(stats.files_deleted as u64, Ordering::Relaxed);
        p.dirs
            .fetch_add(stats.dirs_deleted as u64, Ordering::Relaxed);
        p.bytes.fetch_add(stats.total_size, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Jobs and the worker pool
// ---------------------------------------------------------------------------

struct FileEnt {
    path: PathBuf,
    /// Index of the containing directory in the operand's list, or `NO_DIR`.
    dir: usize,
    size: u64,
    kind: Kind,
}

struct DirEnt {
    path: PathBuf,
    /// Index of the parent directory, or `NO_DIR` for an operand.
    parent: usize,
    /// Listing it failed with this reason; it may still be empty.
    open_err: Option<String>,
}

enum Job {
    Files(Vec<FileEnt>),
    Dirs(Vec<DirEnt>),
}

/// State shared by the walker and the workers.
struct Shared {
    opts: Options,
    errors: Mutex<Vec<String>>,
    stats: Mutex<DeleteStats>,
    /// Directories of the current operand that keep something and therefore
    /// must stay.
    failed: Mutex<Vec<usize>>,
    /// Jobs handed to the pool and not finished yet.
    pending: Mutex<usize>,
    idle: Condvar,
}

impl Shared {
    fn job_done(&self) {
        let mut n = lock(&self.pending);
        *n -= 1;
        if *n == 0 {
            self.idle.notify_all();
        }
    }
}

/// Run one job on the current thread. Results are merged into `sh` once per
/// job, so the locks are taken per batch, not per entry.
fn run_job(sh: &Shared, job: Job) {
    let opts = &sh.opts;
    let verb = if opts.dry_run {
        "would remove"
    } else {
        "removed"
    };
    let mut stats = DeleteStats::new();
    let mut errors = Vec::new();
    let mut failed = Vec::new();
    let mut out = String::new();
    match job {
        Job::Files(ents) => {
            for ent in ents {
                let res = if opts.dry_run {
                    Ok(())
                } else {
                    delete_entry(&ent.path, ent.kind, opts)
                };
                match res {
                    Ok(()) => {
                        stats.files_deleted += 1;
                        stats.total_size += ent.size;
                        if opts.verbose {
                            let _ = writeln!(out, "{verb} '{}'", ent.path.display());
                        }
                    }
                    Err(e) if opts.force && e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => {
                        errors.push(cannot_remove(&ent.path, &reason(&e)));
                        if ent.dir != NO_DIR {
                            failed.push(ent.dir);
                        }
                    }
                }
            }
        }
        Job::Dirs(ents) => {
            for ent in ents {
                match remove_dir(&ent.path, ent.open_err.as_deref(), opts) {
                    Ok(true) => {
                        stats.dirs_deleted += 1;
                        if opts.verbose {
                            let _ = writeln!(out, "{verb} directory '{}'", ent.path.display());
                        }
                    }
                    Ok(false) => {}
                    Err(msg) => {
                        errors.push(msg);
                        if ent.parent != NO_DIR {
                            failed.push(ent.parent);
                        }
                    }
                }
            }
        }
    }
    write_stdout(&out);
    add_progress(opts, &stats);
    lock(&sh.stats).merge(stats);
    if !errors.is_empty() {
        lock(&sh.errors).append(&mut errors);
    }
    if !failed.is_empty() {
        lock(&sh.failed).append(&mut failed);
    }
}

fn worker(sh: &Shared, rx: &Mutex<Receiver<Job>>) {
    loop {
        // The guard is dropped at the end of this statement, before the job
        // runs, so the other workers can take the next one.
        let job = lock(rx).recv();
        let Ok(job) = job else { break };
        run_job(sh, job);
        sh.job_done();
    }
}

struct Pool {
    tx: SyncSender<Job>,
    handles: Vec<JoinHandle<()>>,
}

fn start_pool(sh: &Arc<Shared>) -> Option<Pool> {
    let workers = worker_count(sh.opts.jobs);
    let (tx, rx) = mpsc::sync_channel::<Job>((workers * QUEUE_PER_WORKER).max(1));
    let rx = Arc::new(Mutex::new(rx));
    let mut handles = Vec::with_capacity(workers);
    for i in 0..workers {
        let (sh, rx) = (Arc::clone(sh), Arc::clone(&rx));
        match std::thread::Builder::new()
            .name(format!("rmx-{i}"))
            .spawn(move || worker(&sh, &rx))
        {
            Ok(h) => handles.push(h),
            Err(_) => break,
        }
    }
    // No thread could be created: keep working inline.
    (!handles.is_empty()).then_some(Pool { tx, handles })
}

// ---------------------------------------------------------------------------
// The walker
// ---------------------------------------------------------------------------

/// A directory of the operand being removed.
struct Dir {
    path: PathBuf,
    parent: usize,
    depth: usize,
    /// Listing failed with this reason; rmdir is still attempted.
    open_err: Option<String>,
    /// Already handled (vanished, or turned into a non-directory).
    done: bool,
}

struct Walker {
    sh: Arc<Shared>,
    pool: Option<Pool>,
    pool_tried: bool,
    batch: Vec<FileEnt>,
}

impl Walker {
    fn new(opts: &Options) -> Self {
        Walker {
            sh: Arc::new(Shared {
                opts: *opts,
                errors: Mutex::new(Vec::new()),
                stats: Mutex::new(DeleteStats::new()),
                failed: Mutex::new(Vec::new()),
                pending: Mutex::new(0),
                idle: Condvar::new(),
            }),
            pool: None,
            pool_tried: false,
            batch: Vec::with_capacity(BATCH_ENTRIES),
        }
    }

    fn error(&self, msg: String) {
        lock(&self.sh.errors).push(msg);
    }

    fn fail(&self, dir: usize) {
        if dir != NO_DIR {
            lock(&self.sh.failed).push(dir);
        }
    }

    /// Hand a job to the pool (starting it if `big`), or run it inline.
    fn submit(&mut self, job: Job, big: bool) {
        if big && self.pool.is_none() && !self.pool_tried {
            self.pool_tried = true;
            self.pool = start_pool(&self.sh);
        }
        let Some(pool) = &self.pool else {
            run_job(&self.sh, job);
            return;
        };
        *lock(&self.sh.pending) += 1;
        if let Err(mpsc::SendError(job)) = pool.tx.send(job) {
            // Every worker is gone; do the work here.
            self.sh.job_done();
            run_job(&self.sh, job);
        }
    }

    fn add_file(&mut self, ent: FileEnt) {
        self.batch.push(ent);
        if self.batch.len() >= BATCH_ENTRIES {
            let batch = std::mem::replace(&mut self.batch, Vec::with_capacity(BATCH_ENTRIES));
            self.submit(Job::Files(batch), true);
        }
    }

    /// Submit the partial batch and wait until the pool has finished every job.
    fn settle(&mut self) {
        if !self.batch.is_empty() {
            let batch = std::mem::take(&mut self.batch);
            self.submit(Job::Files(batch), false);
        }
        let mut n = lock(&self.sh.pending);
        while *n > 0 {
            n = self.sh.idle.wait(n).unwrap_or_else(|e| e.into_inner());
        }
    }

    fn finish(mut self) -> Outcome {
        self.settle();
        if let Some(Pool { tx, handles }) = self.pool.take() {
            drop(tx);
            for h in handles {
                let _ = h.join();
            }
        }
        Outcome {
            stats: std::mem::take(&mut *lock(&self.sh.stats)),
            errors: std::mem::take(&mut *lock(&self.sh.errors)),
        }
    }

    fn operand(&mut self, path: &Path) {
        let opts = self.sh.opts;
        if opts.one_file_system {
            self.error(cannot_remove(path, NO_ONE_FS));
            return;
        }
        let meta = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) => {
                if !(opts.force && e.kind() == io::ErrorKind::NotFound) {
                    self.error(cannot_remove(path, &reason(&e)));
                }
                return;
            }
        };
        // is_dir() is false for symlinks and junctions.
        if !meta.is_dir() {
            let size = if opts.track_size { meta.len() } else { 0 };
            self.add_file(FileEnt {
                path: path.to_path_buf(),
                dir: NO_DIR,
                size,
                kind: kind_of(&meta.file_type()),
            });
            return;
        }
        if !opts.recursive {
            if opts.dir {
                let ent = DirEnt {
                    path: path.to_path_buf(),
                    parent: NO_DIR,
                    open_err: None,
                };
                run_job(&self.sh, Job::Dirs(vec![ent]));
            } else {
                self.error(cannot_remove(path, "Is a directory"));
            }
            return;
        }
        self.tree(path);
    }

    /// Remove the directory `root` and everything below it.
    fn tree(&mut self, root: &Path) {
        let mut dirs = vec![Dir {
            path: root.to_path_buf(),
            parent: NO_DIR,
            depth: 0,
            open_err: None,
            done: false,
        }];
        let mut stack = vec![0];
        while let Some(i) = stack.pop() {
            self.list(i, &mut dirs, &mut stack);
        }
        self.settle();

        let mut failed = vec![false; dirs.len()];
        self.take_failed(&mut failed);

        let deepest = dirs.iter().map(|d| d.depth).max().unwrap_or(0);
        let mut levels = vec![Vec::new(); deepest + 1];
        for (i, d) in dirs.iter().enumerate() {
            levels[d.depth].push(i);
        }
        for level in levels.into_iter().rev() {
            let mut todo = Vec::with_capacity(level.len());
            for i in level {
                let d = &mut dirs[i];
                if d.done {
                    continue;
                }
                if failed[i] {
                    if d.parent != NO_DIR {
                        failed[d.parent] = true;
                    }
                    continue;
                }
                todo.push(DirEnt {
                    path: std::mem::take(&mut d.path),
                    parent: d.parent,
                    open_err: d.open_err.take(),
                });
            }
            let big = todo.len() > DIR_CHUNK;
            let mut todo = todo.into_iter().peekable();
            while todo.peek().is_some() {
                let chunk: Vec<DirEnt> = todo.by_ref().take(DIR_CHUNK).collect();
                self.submit(Job::Dirs(chunk), big);
            }
            self.settle();
            self.take_failed(&mut failed);
        }
    }

    /// Move the failures reported since the last call into `failed`.
    fn take_failed(&self, failed: &mut [bool]) {
        for i in std::mem::take(&mut *lock(&self.sh.failed)) {
            failed[i] = true;
        }
    }

    /// List directory `i`: files go to the batch, subdirectories onto `stack`.
    fn list(&mut self, i: usize, dirs: &mut Vec<Dir>, stack: &mut Vec<usize>) {
        let opts = self.sh.opts;
        let path = dirs[i].path.clone();
        let parent = dirs[i].parent;

        // read_dir follows symlinks and junctions, so make sure this is still
        // a real directory (the operand itself was checked a moment ago).
        if i != 0 {
            match fs::symlink_metadata(&path) {
                Ok(m) if m.is_dir() => {}
                Ok(m) => {
                    // Replaced since its parent was listed: remove it as what
                    // it is now.
                    dirs[i].done = true;
                    let size = if opts.track_size { m.len() } else { 0 };
                    self.add_file(FileEnt {
                        path,
                        dir: parent,
                        size,
                        kind: kind_of(&m.file_type()),
                    });
                    return;
                }
                Err(e) => {
                    self.open_failed(i, dirs, e);
                    return;
                }
            }
        }

        let entries = match fs::read_dir(&path) {
            Ok(rd) => rd,
            Err(e) => {
                self.open_failed(i, dirs, e);
                return;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    self.error(format!(
                        "cannot read directory '{}': {}",
                        path.display(),
                        reason(&e)
                    ));
                    self.fail(i);
                    break;
                }
            };
            let child = entry.path();
            // Neither file_type() nor metadata() follows links.
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(e) => {
                    self.error(cannot_remove(&child, &reason(&e)));
                    self.fail(i);
                    continue;
                }
            };
            if ft.is_dir() {
                stack.push(dirs.len());
                dirs.push(Dir {
                    path: child,
                    parent: i,
                    depth: dirs[i].depth + 1,
                    open_err: None,
                    done: false,
                });
            } else {
                let size = if opts.track_size {
                    entry.metadata().map_or(0, |m| m.len())
                } else {
                    0
                };
                self.add_file(FileEnt {
                    path: child,
                    dir: i,
                    size,
                    kind: kind_of(&ft),
                });
            }
        }
    }

    fn open_failed(&mut self, i: usize, dirs: &mut [Dir], e: io::Error) {
        if e.kind() == io::ErrorKind::NotFound {
            dirs[i].done = true;
            if !self.sh.opts.force {
                self.error(cannot_remove(&dirs[i].path, &reason(&e)));
                self.fail(dirs[i].parent);
            }
        } else {
            // Unreadable but possibly empty: rmdir is tried in its turn.
            dirs[i].open_err = Some(reason(&e));
        }
    }
}

/// Remove every path in `paths`. Same contract as the unix engine's
/// `remove_all`; see the module docs for the platform differences.
pub fn remove_all(paths: &[PathBuf], opts: &Options) -> Outcome {
    let mut walker = Walker::new(opts);
    for path in paths {
        walker.operand(path);
    }
    walker.finish()
}

// ---------------------------------------------------------------------------
// Interactive mode
// ---------------------------------------------------------------------------

/// Same contract as the unix engine's `remove_all_interactive`: sequential,
/// `descend into directory` first, one question per entry, then
/// `remove directory` once it has been emptied; a directory that keeps
/// something is skipped silently.
pub fn remove_all_interactive(
    paths: &[PathBuf],
    opts: &Options,
    ask: &mut dyn FnMut(&str) -> bool,
) -> Outcome {
    let mut it = Interactive {
        opts: *opts,
        ask,
        out: Outcome::default(),
    };
    for path in paths {
        it.operand(path);
    }
    it.out
}

struct Interactive<'q> {
    opts: Options,
    ask: &'q mut dyn FnMut(&str) -> bool,
    out: Outcome,
}

struct IFrame {
    path: PathBuf,
    entries: Vec<PathBuf>,
    next: usize,
    /// Something inside was kept, so this directory stays too.
    kept: bool,
}

enum Step {
    Removed,
    Kept,
    Descend(IFrame),
}

impl Interactive<'_> {
    fn missing_ok(&self, e: &io::Error) -> bool {
        self.opts.force && e.kind() == io::ErrorKind::NotFound
    }

    fn verb(&self) -> &'static str {
        if self.opts.dry_run {
            "would remove"
        } else {
            "removed"
        }
    }

    fn counted(&mut self, stats: DeleteStats, line: String) {
        add_progress(&self.opts, &stats);
        self.out.stats.merge(stats);
        if self.opts.verbose {
            write_stdout(&line);
        }
    }

    fn operand(&mut self, path: &Path) {
        if self.opts.one_file_system {
            self.out.errors.push(cannot_remove(path, NO_ONE_FS));
            return;
        }
        let meta = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) => {
                if !self.missing_ok(&e) {
                    self.out.errors.push(cannot_remove(path, &reason(&e)));
                }
                return;
            }
        };
        if !meta.is_dir() {
            self.file(path, &meta);
            return;
        }
        if !self.opts.recursive {
            if !self.opts.dir {
                self.out.errors.push(cannot_remove(path, "Is a directory"));
            } else if (self.ask)(&format!("remove directory '{}'?", path.display())) {
                self.rmdir(path, None);
            }
            return;
        }

        let mut stack = match self.enter(path) {
            Step::Descend(frame) => vec![frame],
            Step::Removed | Step::Kept => return,
        };
        while let Some(top) = stack.last_mut() {
            if top.next < top.entries.len() {
                let child = std::mem::take(&mut top.entries[top.next]);
                top.next += 1;
                match self.entry(&child) {
                    Step::Removed => {}
                    Step::Kept => {
                        if let Some(t) = stack.last_mut() {
                            t.kept = true;
                        }
                    }
                    Step::Descend(frame) => stack.push(frame),
                }
                continue;
            }
            let Some(done) = stack.pop() else { break };
            let removed = !done.kept
                && (self.ask)(&format!("remove directory '{}'?", done.path.display()))
                && self.rmdir(&done.path, None);
            if !removed {
                if let Some(parent) = stack.last_mut() {
                    parent.kept = true;
                }
            }
        }
    }

    /// Ask to descend, then list the directory.
    fn enter(&mut self, path: &Path) -> Step {
        if !(self.ask)(&format!("descend into directory '{}'?", path.display())) {
            return Step::Kept;
        }
        let listing = fs::read_dir(path).and_then(|rd| {
            rd.map(|e| e.map(|e| e.path()))
                .collect::<io::Result<Vec<_>>>()
        });
        match listing {
            Ok(entries) => Step::Descend(IFrame {
                path: path.to_path_buf(),
                entries,
                next: 0,
                kept: false,
            }),
            Err(e) => {
                // Unreadable but possibly empty: offer to remove it anyway.
                let why = reason(&e);
                if (self.ask)(&format!("remove directory '{}'?", path.display()))
                    && self.rmdir(path, Some(&why))
                {
                    Step::Removed
                } else {
                    Step::Kept
                }
            }
        }
    }

    fn entry(&mut self, path: &Path) -> Step {
        let meta = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) => {
                if self.missing_ok(&e) {
                    return Step::Removed;
                }
                self.out.errors.push(cannot_remove(path, &reason(&e)));
                return Step::Kept;
            }
        };
        if meta.is_dir() {
            return self.enter(path);
        }
        if self.file(path, &meta) {
            Step::Removed
        } else {
            Step::Kept
        }
    }

    fn file(&mut self, path: &Path, meta: &fs::Metadata) -> bool {
        let ft = meta.file_type();
        let question = if ft.is_file() {
            format!("remove regular file '{}'?", path.display())
        } else if ft.is_symlink() {
            format!("remove symbolic link '{}'?", path.display())
        } else {
            format!("remove '{}'?", path.display())
        };
        if !(self.ask)(&question) {
            return false;
        }
        let res = if self.opts.dry_run {
            Ok(())
        } else {
            delete_entry(path, kind_of(&ft), &self.opts)
        };
        match res {
            Ok(()) => {
                let stats = DeleteStats {
                    files_deleted: 1,
                    dirs_deleted: 0,
                    total_size: if self.opts.track_size { meta.len() } else { 0 },
                };
                let line = format!("{} '{}'\n", self.verb(), path.display());
                self.counted(stats, line);
                true
            }
            Err(e) if self.missing_ok(&e) => true,
            Err(e) => {
                self.out.errors.push(cannot_remove(path, &reason(&e)));
                false
            }
        }
    }

    fn rmdir(&mut self, path: &Path, open_err: Option<&str>) -> bool {
        match remove_dir(path, open_err, &self.opts) {
            Ok(true) => {
                let stats = DeleteStats {
                    files_deleted: 0,
                    dirs_deleted: 1,
                    total_size: 0,
                };
                let line = format!("{} directory '{}'\n", self.verb(), path.display());
                self.counted(stats, line);
                true
            }
            Ok(false) => true,
            Err(msg) => {
                self.out.errors.push(msg);
                false
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests. They run wherever this module is compiled: on Windows for real, and
// on unix when lib.rs includes it for tests, so the portable engine is
// exercised by the Linux and macOS CI jobs too.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::{remove_all, remove_all_interactive, NO_ONE_FS};
    use crate::{Options, Progress};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            static N: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "rmx-portable-{}-{}-{}",
                tag,
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_file(path: &Path, bytes: usize) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    fn opts() -> Options {
        Options::default()
    }

    fn missing(path: &Path) -> bool {
        fs::symlink_metadata(path).is_err()
    }

    /// Whether permission tests mean anything here, probed rather than
    /// assumed: root ignores permission bits on unix, and wine does not
    /// enforce the read-only attribute on delete the way Windows does.
    fn permissions_enforced() -> bool {
        let t = TempDir::new("permprobe");
        if cfg!(windows) {
            let f = t.path().join("ro");
            fs::write(&f, b"x").unwrap();
            set_readonly(&f, true);
            let enforced = fs::remove_file(&f).is_err();
            if enforced {
                set_readonly(&f, false);
            }
            return enforced;
        }
        let locked = t.path().join("locked");
        fs::create_dir(&locked).unwrap();
        set_readonly(&locked, true);
        let enforced = fs::write(locked.join("probe"), b"x").is_err();
        set_readonly(&locked, false);
        enforced
    }

    #[allow(clippy::permissions_set_readonly_false)] // test scratch only
    fn set_readonly(path: &Path, on: bool) {
        let mut p = fs::metadata(path).unwrap().permissions();
        p.set_readonly(on);
        fs::set_permissions(path, p).unwrap();
    }

    #[test]
    fn removes_a_tree_and_counts_everything() {
        let t = TempDir::new("tree");
        let root = t.path().join("root");
        for d in 0..5 {
            for f in 0..10 {
                write_file(&root.join(format!("d{d}/sub/f{f}")), 10);
            }
        }
        write_file(&root.join("top"), 7);
        let out = remove_all(std::slice::from_ref(&root), &opts());
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert_eq!(out.stats.files_deleted, 51);
        assert_eq!(out.stats.dirs_deleted, 11);
        assert_eq!(out.stats.total_size, 507);
        assert!(missing(&root));
    }

    /// Thousands of files and a level with hundreds of directories: both the
    /// file pool and the per-level directory jobs run on worker threads.
    #[test]
    fn parallel_paths_remove_everything() {
        let t = TempDir::new("wide");
        let root = t.path().join("root");
        for d in 0..300 {
            for f in 0..10 {
                write_file(&root.join(format!("d{d}/f{f}")), 1);
            }
        }
        for f in 0..1000 {
            write_file(&root.join(format!("flat{f}")), 2);
        }
        let progress: &'static Progress = Box::leak(Box::default());
        let o = Options {
            jobs: 4,
            progress: Some(progress),
            ..opts()
        };
        let out = remove_all(std::slice::from_ref(&root), &o);
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert_eq!(out.stats.files_deleted, 4000);
        assert_eq!(out.stats.dirs_deleted, 301);
        assert_eq!(out.stats.total_size, 3000 + 2000);
        assert_eq!(progress.files.load(Ordering::Relaxed), 4000);
        assert_eq!(progress.dirs.load(Ordering::Relaxed), 301);
        assert_eq!(progress.bytes.load(Ordering::Relaxed), 5000);
        assert!(missing(&root));
    }

    /// The walk is iterative: a deep chain must not overflow the stack.
    #[test]
    fn deep_chain_is_removed_without_recursion() {
        let t = TempDir::new("deep");
        let root = t.path().join("r");
        let mut p = root.clone();
        for _ in 0..400 {
            p.push("d");
        }
        fs::create_dir_all(&p).unwrap();
        write_file(&p.join("leaf"), 3);
        let out = remove_all(std::slice::from_ref(&root), &opts());
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert_eq!(out.stats.dirs_deleted, 401);
        assert_eq!(out.stats.files_deleted, 1);
        assert!(missing(&root));
    }

    #[test]
    fn missing_operand_reports_unless_forced() {
        let t = TempDir::new("missing");
        let ghost = t.path().join("ghost");
        let out = remove_all(std::slice::from_ref(&ghost), &opts());
        assert_eq!(
            out.errors,
            vec![format!(
                "cannot remove '{}': No such file or directory",
                ghost.display()
            )]
        );
        let forced = Options {
            force: true,
            ..opts()
        };
        let out = remove_all(std::slice::from_ref(&ghost), &forced);
        assert!(out.errors.is_empty());
        assert_eq!(out.stats.files_deleted, 0);
    }

    #[test]
    fn errors_never_abort_the_run() {
        let t = TempDir::new("continue");
        let a = t.path().join("a");
        let ghost = t.path().join("ghost");
        let b = t.path().join("b");
        write_file(&a, 1);
        write_file(&b, 1);
        let out = remove_all(&[a.clone(), ghost, b.clone()], &opts());
        assert_eq!(out.errors.len(), 1);
        assert_eq!(out.stats.files_deleted, 2);
        assert!(missing(&a) && missing(&b));
    }

    #[test]
    fn directory_without_recursive() {
        let t = TempDir::new("nonrec");
        let full = t.path().join("full");
        let empty = t.path().join("empty");
        write_file(&full.join("x"), 1);
        fs::create_dir(&empty).unwrap();
        let plain = Options {
            recursive: false,
            ..opts()
        };
        let out = remove_all(std::slice::from_ref(&full), &plain);
        assert_eq!(
            out.errors,
            vec![format!(
                "cannot remove '{}': Is a directory",
                full.display()
            )]
        );

        let d = Options {
            recursive: false,
            dir: true,
            ..opts()
        };
        let out = remove_all(&[empty.clone(), full.clone()], &d);
        assert_eq!(out.stats.dirs_deleted, 1);
        assert!(missing(&empty));
        assert_eq!(
            out.errors,
            vec![format!(
                "cannot remove '{}': Directory not empty",
                full.display()
            )]
        );
        assert!(full.join("x").exists());
    }

    #[test]
    fn dry_run_removes_nothing_and_counts_everything() {
        let t = TempDir::new("dry");
        let root = t.path().join("root");
        for f in 0..200 {
            write_file(&root.join(format!("a/b/f{f}")), 5);
        }
        let o = Options {
            dry_run: true,
            jobs: 2,
            ..opts()
        };
        let out = remove_all(std::slice::from_ref(&root), &o);
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert_eq!(out.stats.files_deleted, 200);
        assert_eq!(out.stats.dirs_deleted, 3);
        assert_eq!(out.stats.total_size, 1000);
        assert_eq!(fs::read_dir(root.join("a/b")).unwrap().count(), 200);
    }

    #[test]
    fn one_file_system_is_refused_per_operand() {
        let t = TempDir::new("onefs");
        let a = t.path().join("a");
        let b = t.path().join("b");
        write_file(&a.join("x"), 1);
        write_file(&b, 1);
        let o = Options {
            one_file_system: true,
            ..opts()
        };
        let out = remove_all(&[a.clone(), b.clone()], &o);
        assert_eq!(
            out.errors,
            vec![
                format!("cannot remove '{}': {NO_ONE_FS}", a.display()),
                format!("cannot remove '{}': {NO_ONE_FS}", b.display()),
            ]
        );
        assert!(a.join("x").exists() && b.exists());
        let mut never = |_: &str| -> bool { panic!("must not ask") };
        let out = remove_all_interactive(std::slice::from_ref(&a), &o, &mut never);
        assert_eq!(out.errors.len(), 1);
    }

    #[cfg(unix)]
    fn dir_link(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    /// Creating symlinks needs a privilege (or developer mode) on Windows,
    /// and wine reports success without creating anything; the test is
    /// skipped where no link appears.
    #[cfg(windows)]
    fn dir_link(target: &Path, link: &Path) -> bool {
        std::os::windows::fs::symlink_dir(target, link).is_ok()
            && fs::symlink_metadata(link).is_ok_and(|m| m.file_type().is_symlink())
    }

    #[test]
    fn directory_links_are_removed_not_followed() {
        let t = TempDir::new("links");
        let outside = t.path().join("outside");
        write_file(&outside.join("keep"), 4);
        let root = t.path().join("root");
        write_file(&root.join("f"), 1);
        if !dir_link(&outside, &root.join("link")) {
            eprintln!("skipped: cannot create directory symlinks here");
            return;
        }
        let top_link = t.path().join("toplink");
        assert!(dir_link(&outside, &top_link));

        let out = remove_all(&[root.clone(), top_link.clone()], &opts());
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert!(missing(&root) && missing(&top_link));
        assert!(outside.join("keep").exists());
        assert_eq!(out.stats.files_deleted, 3); // f, link, toplink
        assert_eq!(out.stats.dirs_deleted, 1);
    }

    /// Junctions need no privilege on Windows, which makes them the common
    /// case (npm, pnpm and many installers create them).
    #[cfg(windows)]
    #[test]
    fn junctions_are_removed_not_followed() {
        let t = TempDir::new("junction");
        let outside = t.path().join("outside");
        write_file(&outside.join("keep"), 4);
        let root = t.path().join("root");
        write_file(&root.join("f"), 1);
        let junction = root.join("j");
        let made = std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(&junction)
            .arg(&outside)
            .output()
            .is_ok_and(|o| o.status.success());
        if !made || !fs::symlink_metadata(&junction).is_ok_and(|m| m.file_type().is_symlink()) {
            eprintln!("skipped: cannot create junctions here");
            return;
        }
        let out = remove_all(std::slice::from_ref(&root), &opts());
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert!(missing(&root));
        assert!(outside.join("keep").exists());
        assert_eq!(out.stats.files_deleted, 2); // f, j
    }

    /// A failure deep down keeps every ancestor and reports only the real
    /// error, not "Directory not empty" for each parent. On unix the failure
    /// is a non-writable directory (skipped as root); on Windows it is a
    /// read-only file.
    #[test]
    fn failure_below_keeps_ancestors_quietly() {
        if !permissions_enforced() {
            eprintln!("skipped: permissions are not enforced here (root, wine)");
            return;
        }
        let t = TempDir::new("fail");
        let root = t.path().join("root");
        let stuck = root.join("a/b");
        write_file(&stuck.join("x"), 1);
        write_file(&root.join("other/y"), 1);
        let blocker = if cfg!(windows) {
            stuck.join("x")
        } else {
            stuck.clone()
        };
        set_readonly(&blocker, true);

        let out = remove_all(std::slice::from_ref(&root), &opts());
        assert_eq!(
            out.errors,
            vec![format!(
                "cannot remove '{}': Permission denied",
                stuck.join("x").display()
            )]
        );
        assert!(stuck.join("x").exists());
        assert!(missing(&root.join("other")));

        set_readonly(&blocker, false);
        let out = remove_all(std::slice::from_ref(&root), &opts());
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert!(missing(&root));
    }

    /// Windows only: `force_writable` clears the read-only attribute.
    #[cfg(windows)]
    #[test]
    fn force_writable_clears_readonly_files() {
        if !permissions_enforced() {
            eprintln!("skipped: the read-only attribute is not enforced (wine)");
            return;
        }
        let t = TempDir::new("ro");
        let root = t.path().join("root");
        let file = root.join("objects/ro.pack");
        write_file(&file, 9);
        set_readonly(&file, true);

        let out = remove_all(std::slice::from_ref(&root), &opts());
        assert_eq!(
            out.errors,
            vec![format!(
                "cannot remove '{}': Permission denied",
                file.display()
            )]
        );
        assert!(file.exists());

        let w = Options {
            force_writable: true,
            ..opts()
        };
        let out = remove_all(std::slice::from_ref(&root), &w);
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert_eq!(out.stats.files_deleted, 1);
        assert!(missing(&root));
    }

    #[test]
    fn interactive_asks_in_rm_order() {
        let t = TempDir::new("ask");
        let d = t.path().join("d");
        let f = d.join("a.txt");
        write_file(&f, 3);
        let mut asked = Vec::new();
        let mut yes = |q: &str| {
            asked.push(q.to_string());
            true
        };
        let out = remove_all_interactive(std::slice::from_ref(&d), &opts(), &mut yes);
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert_eq!(
            asked,
            vec![
                format!("descend into directory '{}'?", d.display()),
                format!("remove regular file '{}'?", f.display()),
                format!("remove directory '{}'?", d.display()),
            ]
        );
        assert_eq!(out.stats.files_deleted, 1);
        assert_eq!(out.stats.dirs_deleted, 1);
        assert_eq!(out.stats.total_size, 3);
        assert!(missing(&d));
    }

    #[test]
    fn interactive_declined_entry_keeps_parents_silently() {
        let t = TempDir::new("decline");
        let d = t.path().join("d");
        let keep = d.join("sub/keep");
        let gone = d.join("gone");
        write_file(&keep, 1);
        write_file(&gone, 1);
        let mut asked = Vec::new();
        let mut answer = |q: &str| {
            asked.push(q.to_string());
            !q.contains("keep")
        };
        let out = remove_all_interactive(std::slice::from_ref(&d), &opts(), &mut answer);
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert!(keep.exists());
        assert!(missing(&gone));
        // Neither 'sub' nor 'd' is offered for removal: both keep something.
        assert!(!asked.iter().any(|q| q.starts_with("remove directory")));
    }

    #[test]
    fn interactive_dry_run_keeps_everything() {
        let t = TempDir::new("idry");
        let d = t.path().join("d");
        write_file(&d.join("x"), 2);
        let o = Options {
            dry_run: true,
            ..opts()
        };
        let mut yes = |_: &str| true;
        let out = remove_all_interactive(std::slice::from_ref(&d), &o, &mut yes);
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert_eq!(out.stats.files_deleted, 1);
        assert_eq!(out.stats.dirs_deleted, 1);
        assert!(d.join("x").exists());
    }

    #[test]
    fn interactive_refuses_directory_without_recursive() {
        let t = TempDir::new("inonrec");
        let d = t.path().join("d");
        fs::create_dir(&d).unwrap();
        let o = Options {
            recursive: false,
            ..opts()
        };
        let mut yes = |_: &str| true;
        let out = remove_all_interactive(std::slice::from_ref(&d), &o, &mut yes);
        assert_eq!(
            out.errors,
            vec![format!("cannot remove '{}': Is a directory", d.display())]
        );
        assert!(d.exists());
    }
}
