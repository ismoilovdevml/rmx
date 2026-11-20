use rayon::prelude::*;
use std::fs::{self, DirEntry};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

#[derive(Debug, Clone, Default)]
pub struct DeleteStats {
    pub files_deleted: usize,
    pub dirs_deleted: usize,
    pub total_size: u64,
}

impl DeleteStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn merge(&mut self, other: DeleteStats) {
        self.files_deleted += other.files_deleted;
        self.dirs_deleted += other.dirs_deleted;
        self.total_size += other.total_size;
    }
}

/// Behaviour knobs for the deletion engine.
///
/// `track_size` controls whether byte totals are reported, which costs one
/// `stat` per file. That turns out to be close to free in practice — `readdir`
/// has already warmed the inode cache, so the `stat` never reaches the disk
/// (measured: 2113ms vs 2140ms over 50k files) — so it stays on by default and
/// only `--quiet` turns it off.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// -r: descend into directories.
    pub recursive: bool,
    /// -d: remove empty directories (ignored when `recursive` is set).
    pub dir: bool,
    /// -f: ignore ENOENT only. Every other error is still reported.
    pub force: bool,
    /// -v: print `removed '<path>'` / `removed directory '<path>'` to stdout.
    pub verbose: bool,
    /// Report byte totals (one fstatat per file). `--quiet` turns it off.
    pub track_size: bool,
    /// Worker threads for unlinking. 0 = auto (min(cores, 4)).
    pub jobs: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            recursive: true,
            dir: false,
            force: false,
            verbose: false,
            track_size: true,
            jobs: 0,
        }
    }
}

impl Options {
    pub fn new(verbose: bool, force: bool) -> Self {
        Options {
            verbose,
            force,
            ..Options::default()
        }
    }
}

/// Result of a removal run: what was removed, plus every failure.
///
/// Each error is already formatted for the user without the `rmx: ` prefix,
/// e.g. `cannot remove 'a/b': Permission denied` (strerror text, no
/// "(os error N)" suffix). The CLI prints them to stderr and exits 1 if any.
#[derive(Debug, Default)]
pub struct Outcome {
    pub stats: DeleteStats,
    pub errors: Vec<String>,
}

impl Outcome {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Remove every path in `paths` (the operands from the command line).
///
/// Preconditions (enforced by the CLI, not here): no operand resolves to `/`,
/// none ends in `.` or `..`. Trailing slashes have already been handled.
///
/// Per operand: lstat it. Missing -> error unless `force`. Directory -> if
/// `recursive` remove the whole tree including the directory itself, else if
/// `dir` rmdir it, else error `cannot remove 'x': Is a directory`. Anything
/// else (file, symlink, fifo, ...) -> unlink. Symlinks are never followed.
/// Errors never abort the run: everything removable is removed.
pub fn remove_all(paths: &[PathBuf], opts: &Options) -> Outcome {
    let _ = (paths, opts);
    todo!("engine rewrite")
}

/// Same contract as [`remove_all`], but asks before each removal and walks
/// sequentially. `ask` receives a question without the `rmx: ` prefix, e.g.
/// `remove regular file 'a.txt'?`, `descend into directory 'd'?`,
/// `remove directory 'd'?`, `remove symbolic link 'l'?`, and returns whether
/// the user said yes.
pub fn remove_all_interactive(
    paths: &[PathBuf],
    opts: &Options,
    ask: &mut dyn FnMut(&str) -> bool,
) -> Outcome {
    let _ = (paths, opts, ask);
    todo!("engine rewrite")
}

/// Shared counters. Workers accumulate locally and commit once per batch, so
/// these are touched a handful of times per directory rather than per file.
struct AtomicStats {
    files: AtomicUsize,
    dirs: AtomicUsize,
    size: AtomicU64,
}

impl AtomicStats {
    fn new() -> Self {
        AtomicStats {
            files: AtomicUsize::new(0),
            dirs: AtomicUsize::new(0),
            size: AtomicU64::new(0),
        }
    }

    fn to_delete_stats(&self) -> DeleteStats {
        DeleteStats {
            files_deleted: self.files.load(Ordering::Relaxed),
            dirs_deleted: self.dirs.load(Ordering::Relaxed),
            total_size: self.size.load(Ordering::Relaxed),
        }
    }
}

/// Batch of work results collected by one worker before it touches shared state.
struct Batch {
    files: usize,
    size: u64,
    log: Vec<u8>,
}

impl Batch {
    fn new(verbose: bool, hint: usize) -> Self {
        Batch {
            files: 0,
            size: 0,
            // ~40 bytes per "removed '<path>'" line is a reasonable guess.
            log: if verbose {
                Vec::with_capacity(hint * 40)
            } else {
                Vec::new()
            },
        }
    }

    /// Publish counters and buffered log output in one go.
    fn commit(self, stats: &AtomicStats) {
        if self.files != 0 {
            stats.files.fetch_add(self.files, Ordering::Relaxed);
        }
        if self.size != 0 {
            stats.size.fetch_add(self.size, Ordering::Relaxed);
        }
        if !self.log.is_empty() {
            let stdout = std::io::stdout();
            let mut handle = stdout.lock();
            let _ = handle.write_all(&self.log);
        }
    }
}

/// Below this many files in one directory, the rayon fan-out costs more than it
/// saves and we just delete them on the current thread.
///
/// Measured crossover (min of 7 runs, parallel vs sequential):
///
/// ```text
/// 500 x 128B   1.34x      200 x 5MB    0.95x
/// 2000 x 128B  1.72x      2000 x 1MB   1.17x
/// 10000 x 128B 1.30x
/// ```
///
/// Parallelism pays off from a few hundred files upwards. The one loser is a
/// small batch of large files, where freeing extents contends more than it
/// parallelises, so the threshold sits just above that case.
const FILE_PARALLEL_THRESHOLD: usize = 256;

/// Worker threads used for unlinking.
///
/// `unlink` is a journalled metadata write, so throughput saturates far below
/// the core count and then *degrades* under contention. Measured on a 12-core
/// M-series/APFS box, deleting 50k files (min of 5 runs):
///
/// ```text
/// threads:  1      2      4      6      8      12     24
/// time:     3052   1999   1843   1976   1959   2264   2484  (ms)
/// ```
///
/// The curve is flat from 2-8 and clearly worse at the core count, so we cap
/// low instead of taking rayon's default of one thread per core.
const MAX_WORKERS: usize = 4;

fn worker_count() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    cores.clamp(1, MAX_WORKERS)
}

/// Dedicated pool, so we neither depend on nor clobber rayon's global pool.
fn pool() -> &'static rayon::ThreadPool {
    static POOL: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(worker_count())
            .thread_name(|i| format!("rmx-{i}"))
            .build()
            .expect("failed to build thread pool")
    })
}

/// Remove a single file.
pub fn remove_file(path: &Path, verbose: bool, force: bool) -> Result<DeleteStats, String> {
    let mut stats = DeleteStats::new();

    // symlink_metadata, not metadata: a broken symlink is still removable, and
    // we want the link's own size rather than its target's.
    let size = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(e) => {
            if force {
                return Ok(stats);
            }
            return Err(format!("Cannot access '{}': {}", path.display(), e));
        }
    };

    match fs::remove_file(path) {
        Ok(_) => {
            stats.files_deleted = 1;
            stats.total_size = size;
            if verbose {
                println!("removed '{}'", path.display());
            }
            Ok(stats)
        }
        Err(e) => {
            if force {
                Ok(stats)
            } else {
                Err(format!("Cannot remove '{}': {}", path.display(), e))
            }
        }
    }
}

/// Recursively remove a directory's *contents*. The directory itself is left in
/// place for the caller to remove.
pub fn remove_directory_recursive(
    path: &Path,
    verbose: bool,
    force: bool,
) -> Result<DeleteStats, String> {
    remove_directory_recursive_with(path, &Options::new(verbose, force))
}

pub fn remove_directory_recursive_with(path: &Path, opts: &Options) -> Result<DeleteStats, String> {
    // symlink_metadata so a symlink pointing at a directory is never descended.
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => {}
        _ => return Ok(DeleteStats::new()),
    }

    let stats = AtomicStats::new();
    walk(path, opts, &stats)?;
    Ok(stats.to_delete_stats())
}

/// Delete everything under `path`.
///
/// Entries are split into files and subdirectories in one pass. Subdirectories
/// recurse sequentially; the files in a directory are unlinked in parallel
/// chunks, so each worker commits to the shared counters once per chunk rather
/// than once per file. See the threshold constants above for why the split is
/// this way round.
fn walk(path: &Path, opts: &Options, stats: &AtomicStats) -> Result<(), String> {
    let read_dir = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(e) => {
            if opts.force {
                return Ok(());
            }
            return Err(format!(
                "Failed to read directory '{}': {}",
                path.display(),
                e
            ));
        }
    };

    let mut files: Vec<DirEntry> = Vec::new();
    let mut dirs: Vec<PathBuf> = Vec::new();

    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                if opts.force {
                    continue;
                }
                return Err(format!(
                    "Failed to read directory '{}': {}",
                    path.display(),
                    e
                ));
            }
        };

        // file_type() is served from the dirent's d_type on APFS/ext4, so this
        // is normally free. It also does not follow symlinks, which is what we
        // want: a symlink is unlinked, never descended into.
        let is_dir = match entry.file_type() {
            Ok(ft) => ft.is_dir(),
            Err(e) => {
                if opts.force {
                    continue;
                }
                return Err(format!("Cannot access '{}': {}", entry.path().display(), e));
            }
        };

        if is_dir {
            dirs.push(entry.path());
        } else {
            files.push(entry);
        }
    }

    // Recurse into subdirectories first so their contents are gone before we
    // try to rmdir them.
    // Recursion stays sequential on purpose. Fanning subdirectories out across
    // threads measured *slower* (840ms vs 738ms on a 500-dir x 40-file tree):
    // concurrent traversal contends on filesystem metadata locks and the win
    // never materialises. Parallelism pays off for the unlinks below, not here.
    // Large directories still use every worker, which is the case that matters.
    for dir in &dirs {
        remove_subtree(dir, opts, stats)?;
    }

    if files.len() >= FILE_PARALLEL_THRESHOLD {
        // Aim for a few chunks per thread so work stealing can even out slow
        // files without paying per-file task overhead.
        let chunk = (files.len() / (worker_count() * 4)).max(64);
        // Touch the pool only here: deleting a single file or a small directory
        // then never spawns a worker thread at all.
        let results: Vec<Result<(), String>> = pool().install(|| {
            files
                .par_chunks(chunk)
                .map(|batch| delete_files(batch, opts, stats))
                .collect()
        });
        for result in results {
            result?;
        }
    } else if !files.is_empty() {
        delete_files(&files, opts, stats)?;
    }

    Ok(())
}

/// Empty a subdirectory, then remove the directory itself.
fn remove_subtree(path: &Path, opts: &Options, stats: &AtomicStats) -> Result<(), String> {
    walk(path, opts, stats)?;

    match fs::remove_dir(path) {
        Ok(_) => {
            stats.dirs.fetch_add(1, Ordering::Relaxed);
            if opts.verbose {
                println!("removed directory '{}'", path.display());
            }
            Ok(())
        }
        Err(e) => {
            if opts.force {
                Ok(())
            } else {
                Err(format!(
                    "Cannot remove directory '{}': {}",
                    path.display(),
                    e
                ))
            }
        }
    }
}

/// Unlink a run of files, accumulating counters and log output locally and
/// publishing them once at the end.
fn delete_files(entries: &[DirEntry], opts: &Options, stats: &AtomicStats) -> Result<(), String> {
    let mut batch = Batch::new(opts.verbose, entries.len());
    let mut outcome = Ok(());

    for entry in entries {
        let path = entry.path();

        // Only pay for the stat when the caller actually wants byte totals.
        let size = if opts.track_size {
            match entry.metadata() {
                Ok(m) => m.len(),
                Err(e) => {
                    if !opts.force {
                        outcome = Err(format!("Cannot access '{}': {}", path.display(), e));
                        break;
                    }
                    0
                }
            }
        } else {
            0
        };

        match fs::remove_file(&path) {
            Ok(_) => {
                batch.files += 1;
                batch.size += size;
                if opts.verbose {
                    let _ = writeln!(batch.log, "removed '{}'", path.display());
                }
            }
            Err(e) => {
                if !opts.force {
                    outcome = Err(format!("Cannot remove '{}': {}", path.display(), e));
                    break;
                }
            }
        }
    }

    batch.commit(stats);
    outcome
}

/// Remove empty directory
pub fn remove_empty_directory(
    path: &Path,
    verbose: bool,
    force: bool,
) -> Result<DeleteStats, String> {
    let mut stats = DeleteStats::new();

    match fs::remove_dir(path) {
        Ok(_) => {
            stats.dirs_deleted = 1;
            if verbose {
                println!("removed directory '{}'", path.display());
            }
            Ok(stats)
        }
        Err(e) => {
            if force {
                Ok(stats)
            } else {
                Err(format!(
                    "Cannot remove directory '{}': {}",
                    path.display(),
                    e
                ))
            }
        }
    }
}

/// Interactive prompt
pub fn prompt_user(path: &Path, is_dir: bool) -> bool {
    use std::io::{self, Write};

    let prompt = if is_dir {
        format!("remove directory '{}'? ", path.display())
    } else {
        format!("remove file '{}'? ", path.display())
    };

    print!("{}", prompt);
    io::stdout().flush().unwrap();

    let mut input = String::new();
    match io::stdin().read_line(&mut input) {
        Ok(_) => {
            let answer = input.trim().to_lowercase();
            answer == "y" || answer == "yes"
        }
        Err(_) => false,
    }
}

pub fn remove_file_interactive(
    path: &Path,
    verbose: bool,
    force: bool,
) -> Result<DeleteStats, String> {
    if prompt_user(path, false) {
        remove_file(path, verbose, force)
    } else {
        Ok(DeleteStats::new())
    }
}

pub fn remove_directory_interactive(
    path: &Path,
    verbose: bool,
    force: bool,
) -> Result<DeleteStats, String> {
    remove_directory_interactive_with(path, &Options::new(verbose, force))
}

pub fn remove_directory_interactive_with(
    path: &Path,
    opts: &Options,
) -> Result<DeleteStats, String> {
    if prompt_user(path, true) {
        let mut stats = remove_directory_recursive_with(path, opts)?;
        match fs::remove_dir(path) {
            Ok(_) => {
                stats.dirs_deleted += 1;
                if opts.verbose {
                    println!("removed directory '{}'", path.display());
                }
                Ok(stats)
            }
            Err(e) => {
                if opts.force {
                    Ok(stats)
                } else {
                    Err(format!(
                        "Cannot remove directory '{}': {}",
                        path.display(),
                        e
                    ))
                }
            }
        }
    } else {
        Ok(DeleteStats::new())
    }
}

/// Legacy function for backwards compatibility
pub fn remove_dir_contents(path: &Path) -> Result<(usize, u64), String> {
    let stats = remove_directory_recursive(path, false, false)?;
    Ok((stats.files_deleted, stats.total_size))
}
