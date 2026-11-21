//! The deletion engine.
//!
//! One walker thread (the caller) does an iterative depth-first walk with
//! `openat` + `readdir`. Files are not unlinked by the walker: their names go
//! into a shared batch that spans many small directories, and full batches are
//! handed to a small pool of workers over a bounded queue. Workers `unlinkat`
//! relative to the directory's file descriptor, so no path is ever built on the
//! hot path.
//!
//! Every open directory is an `Arc<DirNode>`. Batches hold a reference to the
//! directories whose entries they carry and every subdirectory holds one to its
//! parent, so the last reference to a directory goes away exactly when it is
//! empty. Dropping that reference closes the descriptor and removes the
//! directory with `unlinkat(parent_fd, name, AT_REMOVEDIR)`. A failure anywhere
//! below marks the directory as failed instead, and the failure travels up to
//! the operand without producing spurious "Directory not empty" errors.
//!
//! Small jobs (a single file, a directory with a handful of entries) never
//! leave the calling thread: the pool is started by the first full batch.
//!
//! Linux serialises unlinks within one directory on the directory's inode
//! lock, so there each directory is listed completely first (large ones are
//! then unlinked in inode order) and only one worker at a time unlinks in it;
//! the parallelism comes from working on many directories at once. APFS has no
//! such lock, so on macOS workers share large directories too.
//!
//! The engine is Unix-only. The public types and the legacy wrappers at the
//! bottom are portable; other platforms get `remove_all` and
//! `remove_all_interactive` from `portable.rs`.

use std::io;
use std::path::Path;

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
    /// -n/--dry-run: walk and count exactly as a real run would, but remove
    /// nothing. Verbose lines read `would remove '<p>'` /
    /// `would remove directory '<p>'`. Stats report what *would* go.
    pub dry_run: bool,
    /// -x/--one-file-system: when recursing, skip any directory whose st_dev
    /// differs from its operand's. Each skip is reported as an error
    /// `skipping '<p>', since it's on a different device` (GNU wording) and
    /// its ancestors are then kept (silently, they are not empty).
    pub one_file_system: bool,
    /// -W/--force-writable: when an unlink/rmdir/open fails with EACCES or
    /// EPERM because the *containing* directory is not writable (or a
    /// directory is not readable/searchable), add u+rwx to that directory
    /// with fchmod/fchmodat and retry once. For read-only caches such as
    /// ~/go/pkg/mod. If the chmod itself fails, report the original error.
    pub force_writable: bool,
    /// Live counters for a progress display. The engine adds to them as work
    /// completes (per batch, not per file); `None` costs nothing.
    pub progress: Option<&'static Progress>,
}

/// Live progress counters, updated by the engine while it runs.
#[derive(Debug, Default)]
pub struct Progress {
    pub files: std::sync::atomic::AtomicU64,
    pub dirs: std::sync::atomic::AtomicU64,
    pub bytes: std::sync::atomic::AtomicU64,
}

impl Progress {
    /// Credit finished work. Called once per batch or per directory, never
    /// per file, so the shared cache lines are touched rarely.
    #[cfg_attr(not(unix), allow(dead_code))]
    fn add(&self, files: u64, dirs: u64, bytes: u64) {
        use std::sync::atomic::Ordering::Relaxed;
        if files != 0 {
            self.files.fetch_add(files, Relaxed);
        }
        if dirs != 0 {
            self.dirs.fetch_add(dirs, Relaxed);
        }
        if bytes != 0 {
            self.bytes.fetch_add(bytes, Relaxed);
        }
    }
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
            dry_run: false,
            one_file_system: false,
            force_writable: false,
            progress: None,
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

/// Rust's io::Error text without the " (os error N)" suffix.
fn strip_os_error(text: String) -> String {
    match text.rfind(" (os error ") {
        Some(i) => text[..i].to_string(),
        None => text,
    }
}

fn io_err_text(e: &io::Error) -> String {
    #[cfg(unix)]
    {
        if let Some(code) = e.raw_os_error() {
            return engine::strerror(code);
        }
    }
    strip_os_error(e.to_string())
}

#[cfg(unix)]
pub use engine::{
    __last_peak_open_dirs, __last_retried_operands, __set_open_dir_budget,
    __set_pretend_other_device, remove_all, remove_all_interactive,
};

#[cfg(unix)]
mod engine {
    use std::cell::Cell;
    use std::collections::{HashSet, VecDeque};
    use std::ffi::{CStr, CString, OsStr};
    use std::fs;
    use std::io::{self, Write};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
    use std::thread::Scope;
    use std::time::Duration;

    use libc::c_int;

    use super::{DeleteStats, Options, Outcome};

    /// Remove every path in `paths` (the operands from the command line).
    ///
    /// Preconditions (enforced by the CLI, not here): no operand resolves to
    /// `/`, none ends in `.` or `..`. Trailing slashes have already been
    /// handled.
    ///
    /// Per operand: lstat it. Missing -> error unless `force` (with `force`, an
    /// operand that cannot exist because a path component is a file, ENOTDIR,
    /// is ignored too, like GNU). Directory -> if `recursive` remove the whole
    /// tree including the directory itself, else if `dir` rmdir it, else error
    /// `cannot remove 'x': Is a directory`. Anything else (file, symlink,
    /// fifo, ...) -> unlink. Symlinks are never followed. Errors never abort
    /// the run: everything removable is removed.
    ///
    /// `dry_run` walks the same tree with the same error and skip rules and
    /// counts what would go, but never unlinks, rmdirs or chmods anything. It
    /// cannot foresee failures of the removal calls themselves (a file in a
    /// read-only directory is counted as removable), and a directory it
    /// cannot open is reported as an error even where a real run might still
    /// remove it (an empty mode-0000 directory, or anything `-W` would make
    /// readable): its contents cannot be listed without changing it. Under
    /// `dir`, a non-empty directory yields `Directory not empty`.
    ///
    /// `force_writable` also applies to the directory that *contains* an
    /// operand when the operand itself cannot be removed from it.
    pub fn remove_all(paths: &[PathBuf], opts: &Options) -> Outcome {
        run(paths, opts)
    }

    /// Same contract as [`remove_all`], but asks before each removal and walks
    /// sequentially. `ask` receives a question without the `rmx: ` prefix,
    /// e.g. `remove regular file 'a.txt'?`, `descend into directory 'd'?`,
    /// `remove directory 'd'?`, `remove symbolic link 'l'?`, and returns
    /// whether the user said yes.
    ///
    /// Directories are handled like GNU `rm -ri`: `descend into directory`
    /// first (no skips the whole subtree), then one question per entry, then
    /// `remove directory` once the directory has been emptied. An empty
    /// directory is only asked about once, with `remove directory`. If
    /// anything inside was kept (declined or failed), the directory cannot be
    /// empty, so it is skipped silently instead of asking and then failing
    /// with ENOTEMPTY.
    ///
    /// With `dry_run` every question is still asked, nothing is removed, and
    /// the outcome reports what the answers would have removed.
    pub fn remove_all_interactive(
        paths: &[PathBuf],
        opts: &Options,
        ask: &mut dyn FnMut(&str) -> bool,
    ) -> Outcome {
        let mut it = Interactive {
            opts: *opts,
            ask,
            out: Outcome::default(),
            links: HashSet::new(),
            dev: 0,
            pretend_xdev: PRETEND_XDEV.with(|c| c.get()),
        };
        for path in paths {
            it.operand(path);
        }
        it.out
    }

    // -----------------------------------------------------------------------
    // Tuning
    // -----------------------------------------------------------------------

    /// Entries per batch handed to a worker. Large enough that the queue lock
    /// and the output lock are touched rarely, small enough that a tree of a
    /// few hundred files still gets spread across workers.
    const BATCH_ENTRIES: usize = 128;

    /// Upper bound for the automatic worker count.
    ///
    /// `unlink` is a journalled metadata write, so throughput saturates far
    /// below the core count and then *degrades* under contention. On a
    /// 12-core M-series/APFS machine 4 workers was the optimum and 6 was
    /// already slower.
    const MAX_AUTO_WORKERS: usize = 4;

    /// Queue depth per worker. Bounds memory and how far the walker runs
    /// ahead.
    const QUEUE_PER_WORKER: usize = 2;

    /// Verbose output is collected and written in chunks of about this size.
    const OUT_FLUSH: usize = 64 * 1024;

    /// Flags for every directory open: never follow a symlink, fail if the
    /// entry stopped being a directory since `readdir` saw it.
    const DIR_FLAGS: c_int =
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;

    /// Let only one worker at a time unlink inside any given directory.
    ///
    /// Linux's VFS takes the parent directory's inode lock exclusively for
    /// every unlink, so workers sharing a directory only fight over that
    /// lock. Measured on XFS, flat 50k files: 1 worker 926ms, 2 workers
    /// 1188ms, 4 workers 1018ms. Directories are still processed in parallel.
    /// APFS has no such lock, and there several workers in one directory is a
    /// clear win.
    const ONE_UNLINKER_PER_DIR: bool = cfg!(any(target_os = "linux", target_os = "android"));

    /// List a whole directory before handing its files out (Linux only).
    ///
    /// `getdents` and `unlink` contend for the same directory lock on Linux,
    /// so interleaving them in one directory costs more than it overlaps.
    /// Listing first also allows the inode sort below. On APFS streaming wins
    /// instead.
    const LIST_THEN_UNLINK: bool = ONE_UNLINKER_PER_DIR;

    /// From this many files, unlink a directory's entries in inode order, as
    /// GNU fts does for large directories: on XFS and ext4 it turns scattered
    /// inode and directory-block updates into mostly sequential ones.
    const INODE_SORT_MIN: usize = 10_000;

    /// Batch slot for an entry that is a top-level operand (relative to the
    /// cwd).
    const NO_DIR: u32 = u32::MAX;

    thread_local! {
        static BUDGET_OVERRIDE: Cell<Option<usize>> = const { Cell::new(None) };
        static LAST_PEAK_OPEN: Cell<usize> = const { Cell::new(0) };
        static LAST_RETRIES: Cell<usize> = const { Cell::new(0) };
        static PRETEND_XDEV: Cell<bool> = const { Cell::new(false) };
    }

    /// Diagnostic hook: how many operands the last non-interactive run
    /// started from the current thread had to walk a second time because a
    /// directory was unexpectedly not empty (see `DirNode::finish`).
    #[doc(hidden)]
    pub fn __last_retried_operands() -> usize {
        LAST_RETRIES.with(|c| c.get())
    }

    /// Test hook: cap the number of simultaneously open directories for runs
    /// started from the current thread. `None` restores the rlimit-derived
    /// value.
    #[doc(hidden)]
    pub fn __set_open_dir_budget(budget: Option<usize>) {
        BUDGET_OVERRIDE.with(|c| c.set(budget));
    }

    /// Test hook: the highest number of simultaneously open directories in
    /// the last non-interactive run started from the current thread.
    #[doc(hidden)]
    pub fn __last_peak_open_dirs() -> usize {
        LAST_PEAK_OPEN.with(|c| c.get())
    }

    /// Test hook: for runs started from the current thread, record every
    /// operand's device as something no directory below it can match, so
    /// that `one_file_system` sees each subdirectory as a mount point. Lets
    /// the skip logic be tested without the privileges to mount anything.
    #[doc(hidden)]
    pub fn __set_pretend_other_device(on: bool) {
        PRETEND_XDEV.with(|c| c.set(on));
    }

    /// How many directories may be open at once.
    ///
    /// macOS starts processes with a soft RLIMIT_NOFILE of 256, which a deep
    /// or wide tree exceeds easily, so the soft limit is first raised towards
    /// the hard limit (macOS rejects anything above OPEN_MAX). A slice is
    /// kept back for stdio, the read descriptor and whatever else the process
    /// has open.
    fn open_dir_budget() -> usize {
        if let Some(b) = BUDGET_OVERRIDE.with(|c| c.get()) {
            return b.max(4);
        }

        #[cfg(target_os = "macos")]
        const WANT: libc::rlim_t = 10240; // OPEN_MAX
        #[cfg(not(target_os = "macos"))]
        const WANT: libc::rlim_t = 65536;

        // SAFETY: plain syscalls on a stack value.
        let soft = unsafe {
            let mut rl: libc::rlimit = std::mem::zeroed();
            if libc::getrlimit(libc::RLIMIT_NOFILE, &mut rl) != 0 {
                return 200;
            }
            let want = WANT.min(rl.rlim_max);
            if rl.rlim_cur < want {
                let raised = libc::rlimit {
                    rlim_cur: want,
                    rlim_max: rl.rlim_max,
                };
                if libc::setrlimit(libc::RLIMIT_NOFILE, &raised) == 0 {
                    rl.rlim_cur = want;
                }
            }
            rl.rlim_cur.min(1 << 16) as usize
        };
        let reserve = (soft / 8).clamp(16, 256);
        soft.saturating_sub(reserve).max(8)
    }

    fn worker_count(jobs: usize) -> usize {
        if jobs > 0 {
            return jobs;
        }
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .clamp(1, MAX_AUTO_WORKERS)
    }

    // -----------------------------------------------------------------------
    // Small libc helpers
    // -----------------------------------------------------------------------

    fn errno() -> c_int {
        io::Error::last_os_error().raw_os_error().unwrap_or(0)
    }

    fn clear_errno() {
        // SAFETY: the errno location is valid thread-local storage.
        unsafe {
            #[cfg(any(
                target_os = "macos",
                target_os = "ios",
                target_os = "freebsd",
                target_os = "dragonfly"
            ))]
            {
                *libc::__error() = 0;
            }
            #[cfg(any(target_os = "linux", target_os = "emscripten"))]
            {
                *libc::__errno_location() = 0;
            }
            #[cfg(any(target_os = "android", target_os = "netbsd", target_os = "openbsd"))]
            {
                *libc::__errno() = 0;
            }
        }
    }

    /// strerror text without Rust's " (os error N)" suffix.
    pub(super) fn strerror(e: c_int) -> String {
        let mut buf = [0 as libc::c_char; 256];
        // SAFETY: buf is writable and its length is passed; XSI strerror_r
        // always NUL-terminates on success.
        let rc = unsafe { libc::strerror_r(e, buf.as_mut_ptr(), buf.len()) };
        if rc == 0 {
            // SAFETY: NUL-terminated by strerror_r.
            let s = unsafe { CStr::from_ptr(buf.as_ptr()) };
            return s.to_string_lossy().into_owned();
        }
        super::strip_os_error(io::Error::from_raw_os_error(e).to_string())
    }

    fn lossy(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    /// `base/name`, without doubling a trailing slash on the operand.
    fn join_into(out: &mut Vec<u8>, name: &[u8]) {
        if !out.is_empty() && out.last() != Some(&b'/') {
            out.push(b'/');
        }
        out.extend_from_slice(name);
    }

    fn lstat_at(dirfd: c_int, name: &CStr) -> Option<libc::stat> {
        // SAFETY: name is NUL-terminated, st is a valid out-pointer.
        unsafe {
            let mut st: libc::stat = std::mem::zeroed();
            if libc::fstatat(dirfd, name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) == 0 {
                Some(st)
            } else {
                None
            }
        }
    }

    fn fstat_fd(fd: c_int) -> Option<libc::stat> {
        // SAFETY: st is a valid out-pointer; a bad fd only yields EBADF.
        unsafe {
            let mut st: libc::stat = std::mem::zeroed();
            if libc::fstat(fd, &mut st) == 0 {
                Some(st)
            } else {
                None
            }
        }
    }

    fn is_dir_mode(st: &libc::stat) -> bool {
        (st.st_mode & libc::S_IFMT) == libc::S_IFDIR
    }

    #[allow(clippy::unnecessary_cast)] // dev_t is i32 on macOS, u64 on Linux
    fn dev_of(st: &libc::stat) -> u64 {
        st.st_dev as u64
    }

    fn open_dir(dirfd: c_int, name: &CStr) -> Result<c_int, c_int> {
        // SAFETY: name is a C string; the caller owns the returned fd.
        let fd = unsafe { libc::openat(dirfd, name.as_ptr(), DIR_FLAGS) };
        if fd >= 0 {
            Ok(fd)
        } else {
            Err(errno())
        }
    }

    fn unlink_at(dirfd: c_int, name: &CStr, flags: c_int) -> Result<(), c_int> {
        // SAFETY: name is a C string; dirfd is open or AT_FDCWD.
        if unsafe { libc::unlinkat(dirfd, name.as_ptr(), flags) } == 0 {
            Ok(())
        } else {
            Err(errno())
        }
    }

    fn is_perm(e: c_int) -> bool {
        e == libc::EACCES || e == libc::EPERM
    }

    /// Run `op`; if it fails with EACCES/EPERM, `enabled` is set (-W) and
    /// `fix` manages to change a permission, run it once more. A failed `fix`
    /// keeps the original error.
    fn with_retry<T>(
        enabled: bool,
        op: impl Fn() -> Result<T, c_int>,
        fix: impl FnOnce() -> bool,
    ) -> Result<T, c_int> {
        match op() {
            Err(e) if enabled && is_perm(e) && fix() => op(),
            r => r,
        }
    }

    /// -W: add u+rwx to the open directory `fd`. False if that changes
    /// nothing (already rwx) or the chmod fails.
    fn add_rwx_fd(fd: c_int) -> bool {
        let Some(st) = fstat_fd(fd) else {
            return false;
        };
        if st.st_mode & libc::S_IRWXU == libc::S_IRWXU {
            return false;
        }
        let mode = (st.st_mode & 0o7777) | libc::S_IRWXU;
        // SAFETY: plain syscall on an open descriptor.
        unsafe { libc::fchmod(fd, mode) == 0 }
    }

    /// -W: add u+rwx to the directory `name` in `dirfd`. Entries inside the
    /// tree are never followed if they are symlinks; `follow` is for paths
    /// the user named (an operand's parent).
    fn add_rwx_at(dirfd: c_int, name: &CStr, follow: bool) -> bool {
        let st = if follow {
            // SAFETY: name is a C string, st a valid out-pointer.
            unsafe {
                let mut st: libc::stat = std::mem::zeroed();
                (libc::fstatat(dirfd, name.as_ptr(), &mut st, 0) == 0).then_some(st)
            }
        } else {
            lstat_at(dirfd, name)
        };
        let Some(st) = st else {
            return false;
        };
        if !is_dir_mode(&st) || st.st_mode & libc::S_IRWXU == libc::S_IRWXU {
            return false;
        }
        let mode = (st.st_mode & 0o7777) | libc::S_IRWXU;
        let flags = if follow { 0 } else { libc::AT_SYMLINK_NOFOLLOW };
        // SAFETY: name is a C string.
        if unsafe { libc::fchmodat(dirfd, name.as_ptr(), mode, flags) } == 0 {
            return true;
        }
        let e = errno();
        // Older glibc (or no /proc) cannot chmod without following. The entry
        // was a directory a moment ago, so follow it.
        if flags != 0 && (e == libc::EOPNOTSUPP || e == libc::ENOTSUP) {
            // SAFETY: as above.
            return unsafe { libc::fchmodat(dirfd, name.as_ptr(), mode, 0) } == 0;
        }
        false
    }

    /// The directory containing `path` ("." for a bare name).
    fn parent_dir(path: &[u8]) -> CString {
        let parent = Path::new(OsStr::from_bytes(path))
            .parent()
            .map(|p| p.as_os_str().as_bytes())
            .filter(|p| !p.is_empty())
            .unwrap_or(b".");
        CString::new(parent).unwrap_or_default()
    }

    fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn write_stdout(bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let stdout = io::stdout();
        let mut handle = stdout.lock();
        let _ = handle.write_all(bytes);
    }

    // -----------------------------------------------------------------------
    // Shared run state
    // -----------------------------------------------------------------------

    struct Ctx {
        opts: Options,
        /// First pass: an unexplained ENOTEMPTY schedules the operand for one
        /// more pass instead of being reported (see `DirNode::finish`).
        allow_retry: bool,
        /// See `__set_pretend_other_device`.
        pretend_xdev: bool,
        errors: Mutex<Vec<String>>,
        stats: Mutex<DeleteStats>,
        dirs_removed: AtomicUsize,
        /// (dev, ino) of multiply-linked files already counted towards size.
        links: Mutex<HashSet<(u64, u64)>>,
        has_links: AtomicBool,
        /// Directories currently open (one descriptor each).
        live: AtomicUsize,
        live_lock: Mutex<()>,
        live_cv: Condvar,
        walker_waiting: AtomicBool,
        retry: Mutex<Vec<usize>>,
        /// Verbose lines not yet written, in the order they were produced.
        /// A directory is finished only after every batch holding its files
        /// has appended its lines, and only after its subdirectories are
        /// finished, so its own line always follows theirs.
        out: Mutex<Vec<u8>>,
    }

    impl Ctx {
        fn new(opts: Options) -> Self {
            Ctx {
                opts,
                allow_retry: true,
                pretend_xdev: PRETEND_XDEV.with(|c| c.get()),
                errors: Mutex::new(Vec::new()),
                stats: Mutex::new(DeleteStats::new()),
                dirs_removed: AtomicUsize::new(0),
                links: Mutex::new(HashSet::new()),
                has_links: AtomicBool::new(false),
                live: AtomicUsize::new(0),
                live_lock: Mutex::new(()),
                live_cv: Condvar::new(),
                walker_waiting: AtomicBool::new(false),
                retry: Mutex::new(Vec::new()),
                out: Mutex::new(Vec::new()),
            }
        }

        fn error(&self, msg: String) {
            lock(&self.errors).push(msg);
        }

        fn error_at(&self, verb: &str, path: &[u8], e: c_int) {
            self.error(format!("{verb} '{}': {}", lossy(path), strerror(e)));
        }

        /// -W is on and allowed to change anything.
        fn fix_perms(&self) -> bool {
            self.opts.force_writable && !self.opts.dry_run
        }

        /// Bytes to credit for a removed entry. Hard links share one inode,
        /// so a multiply-linked file is counted once (pnpm stores make this
        /// common).
        #[allow(clippy::unnecessary_cast)] // field types differ between platforms
        fn size_of(&self, st: &libc::stat) -> u64 {
            let size = st.st_size.max(0) as u64;
            let key = (st.st_dev as u64, st.st_ino as u64);
            if st.st_nlink as u64 > 1 {
                self.has_links.store(true, Ordering::Relaxed);
                if lock(&self.links).insert(key) {
                    size
                } else {
                    0
                }
            } else if self.has_links.load(Ordering::Relaxed) && lock(&self.links).contains(&key) {
                // The other links are already gone, so nlink dropped to 1.
                0
            } else {
                size
            }
        }

        fn dir_closed(&self) {
            self.live.fetch_sub(1, Ordering::AcqRel);
            if self.walker_waiting.load(Ordering::Acquire) {
                let _g = lock(&self.live_lock);
                self.live_cv.notify_one();
            }
        }

        /// Queue verbose output; it is written in large chunks.
        fn emit(&self, bytes: &[u8]) {
            if bytes.is_empty() {
                return;
            }
            let mut out = lock(&self.out);
            out.extend_from_slice(bytes);
            if out.len() >= OUT_FLUSH {
                // Written under the lock so chunks keep their order.
                write_stdout(&out);
                out.clear();
            }
        }

        fn flush_out(&self) {
            let mut out = lock(&self.out);
            write_stdout(&out);
            out.clear();
        }

        /// Account for a removed (or, in a dry run, removable) directory.
        fn dir_removed(&self, path: impl FnOnce() -> Vec<u8>) {
            self.dirs_removed.fetch_add(1, Ordering::Relaxed);
            if let Some(p) = self.opts.progress {
                p.add(0, 1, 0);
            }
            if self.opts.verbose {
                let mut line = if self.opts.dry_run {
                    b"would remove directory '".to_vec()
                } else {
                    b"removed directory '".to_vec()
                };
                line.extend_from_slice(&path());
                line.extend_from_slice(b"'\n");
                self.emit(&line);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Directory nodes
    // -----------------------------------------------------------------------

    /// An open directory. Removed from its parent when the last reference
    /// drops.
    struct DirNode<'a> {
        ctx: &'a Ctx,
        fd: c_int,
        /// The directory containing this one; `None` for an operand.
        parent: Option<Arc<DirNode<'a>>>,
        /// Name in `parent`, or the operand path for an operand.
        name: CString,
        /// Index of the operand this directory belongs to.
        root: usize,
        /// st_dev of the operand (for `one_file_system`).
        dev: u64,
        /// Something below could not be removed; skip our own rmdir.
        failed: AtomicBool,
        /// Held while a worker unlinks in this directory (Linux only, see
        /// `ONE_UNLINKER_PER_DIR`).
        unlinking: Mutex<()>,
        /// -W: outcome of the one attempt to add u+rwx to this directory.
        /// Whichever thread hits EACCES first does the chmod; the others wait
        /// for it and reuse the result.
        writable: OnceLock<bool>,
    }

    impl<'a> DirNode<'a> {
        fn new(
            ctx: &'a Ctx,
            fd: c_int,
            parent: Option<Arc<DirNode<'a>>>,
            name: CString,
            root: usize,
            dev: u64,
        ) -> Arc<Self> {
            Arc::new(DirNode {
                ctx,
                fd,
                parent,
                name,
                root,
                dev,
                failed: AtomicBool::new(false),
                unlinking: Mutex::new(()),
                writable: OnceLock::new(),
            })
        }

        fn fail(&self) {
            self.failed.store(true, Ordering::Relaxed);
        }

        /// Display path: the operand joined with every name below it.
        fn path(&self) -> Vec<u8> {
            let mut names: Vec<&[u8]> = vec![self.name.to_bytes()];
            let mut cur = self;
            while let Some(p) = &cur.parent {
                names.push(p.name.to_bytes());
                cur = p;
            }
            let mut out = Vec::with_capacity(names.iter().map(|n| n.len() + 1).sum());
            for name in names.iter().rev() {
                join_into(&mut out, name);
            }
            out
        }

        fn child_path(&self, name: &[u8]) -> Vec<u8> {
            let mut p = self.path();
            join_into(&mut p, name);
            p
        }

        /// -W: add u+rwx to this directory, at most once per run.
        fn make_writable(&self) -> bool {
            *self.writable.get_or_init(|| add_rwx_fd(self.fd))
        }

        /// -W: make the directory that contains this one writable.
        fn make_container_writable(&self) -> bool {
            match &self.parent {
                Some(p) => p.make_writable(),
                None => add_rwx_at(libc::AT_FDCWD, &parent_dir(self.name.to_bytes()), true),
            }
        }

        fn rmdir_self(&self) -> Result<(), c_int> {
            let dirfd = self.parent.as_ref().map_or(libc::AT_FDCWD, |p| p.fd);
            unlink_at(dirfd, &self.name, libc::AT_REMOVEDIR)
        }

        /// Close the descriptor and remove the directory (unless something
        /// below failed). Returns the parent reference so the caller can
        /// release it without recursing. Idempotent.
        fn finish(&mut self) -> Option<Arc<DirNode<'a>>> {
            if self.fd < 0 {
                return None;
            }
            let ctx = self.ctx;
            // SAFETY: we own fd and close it exactly once.
            unsafe { libc::close(self.fd) };
            self.fd = -1;
            ctx.dir_closed();

            if self.failed.load(Ordering::Relaxed) {
                if let Some(p) = &self.parent {
                    p.fail();
                }
                return self.parent.take();
            }

            let res = if ctx.opts.dry_run {
                Ok(())
            } else {
                with_retry(
                    ctx.fix_perms(),
                    || self.rmdir_self(),
                    || self.make_container_writable(),
                )
            };
            match res {
                Ok(()) => ctx.dir_removed(|| self.path()),
                Err(e) => {
                    if e == libc::ENOENT && ctx.opts.force {
                        // Someone else removed it; nothing to report.
                    } else if (e == libc::ENOTEMPTY || e == libc::EEXIST) && ctx.allow_retry {
                        // Nothing below failed, yet the directory is not
                        // empty: either an entry appeared concurrently or
                        // readdir skipped one while entries were being
                        // unlinked. Walk the operand again instead of
                        // reporting; the second pass reports for real.
                        lock(&ctx.retry).push(self.root);
                        if let Some(p) = &self.parent {
                            p.fail();
                        }
                    } else {
                        ctx.error_at("cannot remove", &self.path(), e);
                        if let Some(p) = &self.parent {
                            p.fail();
                        }
                    }
                }
            }
            self.parent.take()
        }
    }

    impl Drop for DirNode<'_> {
        fn drop(&mut self) {
            // Release the parent chain iteratively: a deep tree emptied by
            // its last batch must not recurse once per level.
            let mut next = self.finish();
            while let Some(parent) = next {
                next = match Arc::into_inner(parent) {
                    Some(mut node) => node.finish(),
                    None => None,
                };
            }
        }
    }

    // -----------------------------------------------------------------------
    // Batches and the worker queue
    // -----------------------------------------------------------------------

    /// Up to `BATCH_ENTRIES` names from any number of directories. Names live
    /// in one NUL-separated buffer instead of one allocation each.
    struct Batch<'a> {
        dirs: Vec<Arc<DirNode<'a>>>,
        /// (index into `dirs` or `NO_DIR`, offset into `names`)
        ents: Vec<(u32, u32)>,
        names: Vec<u8>,
    }

    impl<'a> Batch<'a> {
        fn new() -> Self {
            Batch {
                dirs: Vec::new(),
                ents: Vec::with_capacity(BATCH_ENTRIES),
                names: Vec::with_capacity(BATCH_ENTRIES * 24),
            }
        }

        fn len(&self) -> usize {
            self.ents.len()
        }

        fn push(&mut self, dir: Option<&Arc<DirNode<'a>>>, name: &[u8]) {
            let d = match dir {
                Some(node) => {
                    if !self.dirs.last().is_some_and(|last| Arc::ptr_eq(last, node)) {
                        self.dirs.push(Arc::clone(node));
                    }
                    (self.dirs.len() - 1) as u32
                }
                None => NO_DIR,
            };
            let off = self.names.len() as u32;
            self.names.extend_from_slice(name);
            self.names.push(0);
            self.ents.push((d, off));
        }
    }

    /// Unlink every entry of a batch, then release its directory references
    /// (which may remove directories that became empty). In a dry run the
    /// entries are only counted.
    fn process_batch(ctx: &Ctx, batch: Batch<'_>, stats: &mut DeleteStats, out: &mut Vec<u8>) {
        let opts = ctx.opts;
        let fix_perms = ctx.fix_perms();
        let mut files = 0usize;
        let mut bytes = 0u64;
        // Entries of one directory are contiguous in a batch, so the lock is
        // taken once per directory run, not per file.
        let mut held: Option<(u32, MutexGuard<'_, ()>)> = None;
        for &(d, off) in &batch.ents {
            let tail = &batch.names[off as usize..];
            let Ok(name) = CStr::from_bytes_until_nul(tail) else {
                continue;
            };
            let node = if d == NO_DIR {
                None
            } else {
                Some(&batch.dirs[d as usize])
            };
            if ONE_UNLINKER_PER_DIR && !opts.dry_run && held.as_ref().map(|h| h.0) != Some(d) {
                held = None;
                if let Some(n) = node {
                    held = Some((d, lock(&n.unlinking)));
                }
            }
            let dirfd = node.map_or(libc::AT_FDCWD, |n| n.fd);

            let size = if opts.track_size {
                lstat_at(dirfd, name).map_or(0, |st| ctx.size_of(&st))
            } else {
                0
            };

            let res = if opts.dry_run {
                Ok(())
            } else {
                with_retry(
                    fix_perms,
                    || unlink_at(dirfd, name, 0),
                    || match node {
                        Some(n) => n.make_writable(),
                        None => add_rwx_at(libc::AT_FDCWD, &parent_dir(name.to_bytes()), true),
                    },
                )
            };
            match res {
                Ok(()) => {
                    files += 1;
                    bytes += size;
                    if opts.verbose {
                        out.extend_from_slice(if opts.dry_run {
                            b"would remove '"
                        } else {
                            b"removed '"
                        });
                        match node {
                            Some(n) => out.extend_from_slice(&n.child_path(name.to_bytes())),
                            None => out.extend_from_slice(name.to_bytes()),
                        }
                        out.extend_from_slice(b"'\n");
                    }
                }
                Err(e) => {
                    if e == libc::ENOENT && opts.force {
                        continue;
                    }
                    let path = match node {
                        Some(n) => n.child_path(name.to_bytes()),
                        None => name.to_bytes().to_vec(),
                    };
                    ctx.error_at("cannot remove", &path, e);
                    if let Some(n) = node {
                        n.fail();
                    }
                }
            }
        }
        drop(held);
        stats.files_deleted += files;
        stats.total_size += bytes;
        if let Some(p) = opts.progress {
            p.add(files as u64, 0, bytes);
        }
        // Queue the lines before the directory references go, so a
        // directory's own line comes after its files.
        ctx.emit(out);
        out.clear();
        drop(batch);
    }

    struct QueueState<'a> {
        items: VecDeque<Batch<'a>>,
        closed: bool,
    }

    struct Queue<'a> {
        state: Mutex<QueueState<'a>>,
        not_empty: Condvar,
        not_full: Condvar,
        /// Set when the pool starts; sizing it needs the worker count, which
        /// is not free to compute (cgroup files on Linux) and not needed
        /// otherwise.
        cap: AtomicUsize,
    }

    impl<'a> Queue<'a> {
        fn new() -> Self {
            Queue {
                state: Mutex::new(QueueState {
                    items: VecDeque::new(),
                    closed: false,
                }),
                not_empty: Condvar::new(),
                not_full: Condvar::new(),
                cap: AtomicUsize::new(1),
            }
        }

        fn push(&self, batch: Batch<'a>) {
            let cap = self.cap.load(Ordering::Relaxed);
            let mut st = lock(&self.state);
            while st.items.len() >= cap {
                st = self.not_full.wait(st).unwrap_or_else(|e| e.into_inner());
            }
            st.items.push_back(batch);
            drop(st);
            self.not_empty.notify_one();
        }

        fn pop(&self) -> Option<Batch<'a>> {
            let mut st = lock(&self.state);
            loop {
                if let Some(b) = st.items.pop_front() {
                    drop(st);
                    self.not_full.notify_one();
                    return Some(b);
                }
                if st.closed {
                    return None;
                }
                st = self.not_empty.wait(st).unwrap_or_else(|e| e.into_inner());
            }
        }

        fn close(&self) {
            lock(&self.state).closed = true;
            self.not_empty.notify_all();
        }
    }

    fn worker(ctx: &Ctx, queue: &Queue<'_>) {
        let mut stats = DeleteStats::new();
        let mut out = Vec::new();
        while let Some(batch) = queue.pop() {
            process_batch(ctx, batch, &mut stats, &mut out);
        }
        lock(&ctx.stats).merge(stats);
    }

    /// Dry run of `rmdir`: Ok if `name` is an empty directory.
    fn check_empty_dir(dirfd: c_int, name: &CStr) -> Result<(), c_int> {
        let fd = open_dir(dirfd, name)?;
        // SAFETY: fd is ours; on success the DIR owns it.
        let dirp = unsafe { libc::fdopendir(fd) };
        if dirp.is_null() {
            let e = errno();
            // SAFETY: fdopendir failed, so fd is still ours.
            unsafe { libc::close(fd) };
            return Err(e);
        }
        let mut res = Ok(());
        loop {
            clear_errno();
            // SAFETY: dirp is a valid open DIR.
            let ent = unsafe { libc::readdir(dirp) };
            if ent.is_null() {
                let e = errno();
                if e != 0 {
                    res = Err(e);
                }
                break;
            }
            // SAFETY: valid entry with a NUL-terminated name.
            let entry = unsafe { CStr::from_ptr((*ent).d_name.as_ptr()) }.to_bytes();
            if entry != b"." && entry != b".." {
                res = Err(libc::ENOTEMPTY);
                break;
            }
        }
        // SAFETY: closes the DIR and fd, once.
        unsafe { libc::closedir(dirp) };
        res
    }

    // -----------------------------------------------------------------------
    // The walker
    // -----------------------------------------------------------------------

    struct Frame<'a> {
        node: Arc<DirNode<'a>>,
        subdirs: Vec<CString>,
        next: usize,
    }

    struct Walker<'scope, 'env, 'c: 'env> {
        ctx: &'c Ctx,
        queue: &'env Queue<'c>,
        scope: &'scope Scope<'scope, 'env>,
        budget: usize,
        peak: usize,
        batch: Batch<'c>,
        /// Workers started (false: everything so far ran on this thread).
        pooled: bool,
        stats: DeleteStats,
        out: Vec<u8>,
        /// `LIST_THEN_UNLINK` scratch, reused across directories:
        /// (inode, offset into `listed_names`).
        listed: Vec<(libc::ino_t, u32)>,
        listed_names: Vec<u8>,
    }

    impl<'scope, 'env, 'c: 'env> Walker<'scope, 'env, 'c> {
        fn add_file(&mut self, dir: Option<&Arc<DirNode<'c>>>, name: &[u8]) {
            self.batch.push(dir, name);
            if self.batch.len() >= BATCH_ENTRIES {
                self.submit();
            }
        }

        /// Hand the current batch to the pool, starting it on first use.
        fn submit(&mut self) {
            if self.batch.len() == 0 {
                return;
            }
            let batch = std::mem::replace(&mut self.batch, Batch::new());
            if !self.pooled {
                self.start_pool();
            }
            if self.pooled {
                self.queue.push(batch);
            } else {
                process_batch(self.ctx, batch, &mut self.stats, &mut self.out);
            }
        }

        /// Push out the partial batch: to the pool if it is running, else
        /// inline.
        fn flush(&mut self) {
            if self.batch.len() == 0 {
                return;
            }
            if self.pooled {
                self.submit();
            } else {
                let batch = std::mem::replace(&mut self.batch, Batch::new());
                process_batch(self.ctx, batch, &mut self.stats, &mut self.out);
            }
        }

        fn start_pool(&mut self) {
            let (ctx, queue) = (self.ctx, self.queue);
            let workers = worker_count(ctx.opts.jobs);
            queue
                .cap
                .store((workers * QUEUE_PER_WORKER).max(1), Ordering::Relaxed);
            for i in 0..workers {
                let spawned = std::thread::Builder::new()
                    .name(format!("rmx-{i}"))
                    .spawn_scoped(self.scope, move || worker(ctx, queue));
                if spawned.is_ok() {
                    self.pooled = true;
                } else {
                    break;
                }
            }
            // If no thread could be created, keep working inline.
        }

        fn finish(mut self) {
            self.flush();
            self.queue.close();
            lock(&self.ctx.stats).merge(std::mem::take(&mut self.stats));
            LAST_PEAK_OPEN.with(|c| c.set(c.get().max(self.peak)));
        }

        fn opened(&mut self) {
            let live = self.ctx.live.fetch_add(1, Ordering::AcqRel) + 1;
            self.peak = self.peak.max(live);
        }

        /// Wait until another directory may be opened. Directories held only
        /// by queued batches are released as workers get through them; the
        /// ones on our own stack are not, so once they alone fill the budget
        /// we go ahead and let `openat` report EMFILE rather than wait
        /// forever.
        fn reserve_slot(&mut self, on_stack: usize) {
            let ctx = self.ctx;
            if ctx.live.load(Ordering::Acquire) < self.budget {
                return;
            }
            self.flush();
            if !self.pooled {
                return;
            }
            let mut last = usize::MAX;
            let mut idle_rounds = 0;
            loop {
                let live = ctx.live.load(Ordering::Acquire);
                if live < self.budget || live <= on_stack {
                    break;
                }
                if live == last {
                    idle_rounds += 1;
                    // No progress for ~10s: give up waiting rather than hang.
                    if idle_rounds > 1000 {
                        break;
                    }
                } else {
                    idle_rounds = 0;
                    last = live;
                }
                let guard = lock(&ctx.live_lock);
                ctx.walker_waiting.store(true, Ordering::Release);
                if ctx.live.load(Ordering::Acquire) < self.budget {
                    break;
                }
                let _ = ctx.live_cv.wait_timeout(guard, Duration::from_millis(10));
            }
            ctx.walker_waiting.store(false, Ordering::Release);
        }

        fn operand(&mut self, index: usize, path: &Path) {
            let ctx = self.ctx;
            let opts = ctx.opts;
            let bytes = path.as_os_str().as_bytes();
            let Ok(cpath) = CString::new(bytes) else {
                ctx.error_at("cannot remove", bytes, libc::EINVAL);
                return;
            };

            let Some(st) = lstat_at(libc::AT_FDCWD, &cpath) else {
                let e = errno();
                // GNU -f is also silent when a path component is a file
                // (`rm -f file/x`).
                let missing = e == libc::ENOENT || e == libc::ENOTDIR;
                if !(missing && opts.force) {
                    ctx.error_at("cannot remove", bytes, e);
                }
                return;
            };

            if !is_dir_mode(&st) {
                self.add_file(None, bytes);
                return;
            }

            if !opts.recursive {
                if opts.dir {
                    self.rmdir_operand(&cpath);
                } else {
                    ctx.error_at("cannot remove", bytes, libc::EISDIR);
                }
                return;
            }

            self.reserve_slot(0);
            let opened = with_retry(
                ctx.fix_perms(),
                || open_dir(libc::AT_FDCWD, &cpath),
                || add_rwx_at(libc::AT_FDCWD, &cpath, false),
            );
            let fd = match opened {
                Ok(fd) => fd,
                Err(e) => {
                    match e {
                        // Replaced by a symlink or file since the lstat.
                        libc::ENOTDIR | libc::ELOOP => self.add_file(None, bytes),
                        libc::ENOENT if opts.force => {}
                        libc::ENOENT => ctx.error_at("cannot remove", bytes, e),
                        // Unreadable but possibly empty (mode 0000): rmdir
                        // may work. A dry run cannot tell.
                        _ => {
                            if opts.dry_run || self.rmdir_top(&cpath).is_err() {
                                ctx.error_at("cannot remove", bytes, e);
                            } else {
                                ctx.dir_removed(|| bytes.to_vec());
                            }
                        }
                    }
                    return;
                }
            };
            self.opened();
            let dev = if ctx.pretend_xdev {
                !dev_of(&st)
            } else {
                dev_of(&st)
            };
            let root = DirNode::new(ctx, fd, None, cpath, index, dev);
            self.walk(root);
        }

        /// `rmdir` an operand, with -W help for its parent directory.
        fn rmdir_top(&self, cpath: &CStr) -> Result<(), c_int> {
            with_retry(
                self.ctx.fix_perms(),
                || unlink_at(libc::AT_FDCWD, cpath, libc::AT_REMOVEDIR),
                || add_rwx_at(libc::AT_FDCWD, &parent_dir(cpath.to_bytes()), true),
            )
        }

        /// `rmdir` an operand (the `-d` case), reporting failure.
        fn rmdir_operand(&mut self, cpath: &CStr) {
            let ctx = self.ctx;
            let res = if ctx.opts.dry_run {
                check_empty_dir(libc::AT_FDCWD, cpath)
            } else {
                self.rmdir_top(cpath)
            };
            match res {
                Ok(()) => ctx.dir_removed(|| cpath.to_bytes().to_vec()),
                Err(e) => {
                    if !(e == libc::ENOENT && ctx.opts.force) {
                        ctx.error_at("cannot remove", cpath.to_bytes(), e);
                    }
                }
            }
        }

        /// Iterative depth-first walk below `root`.
        fn walk(&mut self, root: Arc<DirNode<'c>>) {
            let ctx = self.ctx;
            let subdirs = self.read_dir(&root);
            let mut stack = vec![Frame {
                node: root,
                subdirs,
                next: 0,
            }];

            while let Some(frame) = stack.last_mut() {
                if frame.next >= frame.subdirs.len() {
                    // Done listing this directory. Dropping the frame
                    // releases the walker's reference; the directory goes
                    // once its last batch and subdirectory are finished.
                    stack.pop();
                    continue;
                }
                let name = std::mem::take(&mut frame.subdirs[frame.next]);
                frame.next += 1;
                let parent = Arc::clone(&frame.node);

                self.reserve_slot(stack.len());
                let opened = with_retry(
                    ctx.fix_perms(),
                    || open_dir(parent.fd, &name),
                    // The parent may lack search permission, the child read
                    // or search permission: fix both (each at most once).
                    || parent.make_writable() | add_rwx_at(parent.fd, &name, false),
                );
                let fd = match opened {
                    Ok(fd) => fd,
                    Err(e) => {
                        self.open_failed(&parent, &name, e);
                        continue;
                    }
                };
                if ctx.opts.one_file_system
                    && !fstat_fd(fd).is_none_or(|st| dev_of(&st) == parent.dev)
                {
                    // SAFETY: fd was just opened and is not shared.
                    unsafe { libc::close(fd) };
                    ctx.error(format!(
                        "skipping '{}', since it's on a different device",
                        lossy(&parent.child_path(name.to_bytes()))
                    ));
                    parent.fail();
                    continue;
                }
                self.opened();
                let (root, dev) = (parent.root, parent.dev);
                let node = DirNode::new(ctx, fd, Some(parent), name, root, dev);
                let subdirs = self.read_dir(&node);
                stack.push(Frame {
                    node,
                    subdirs,
                    next: 0,
                });
            }
        }

        fn open_failed(&mut self, parent: &Arc<DirNode<'c>>, name: &CStr, e: c_int) {
            let ctx = self.ctx;
            match e {
                // Became a file or symlink after readdir: unlink it as one.
                libc::ENOTDIR | libc::ELOOP => self.add_file(Some(parent), name.to_bytes()),
                libc::ENOENT if ctx.opts.force => {}
                libc::ENOENT => {
                    ctx.error_at("cannot remove", &parent.child_path(name.to_bytes()), e);
                    parent.fail();
                }
                _ => {
                    // Unreadable (e.g. mode 0000) but maybe empty: try rmdir.
                    // A dry run cannot tell.
                    let removed = !ctx.opts.dry_run
                        && with_retry(
                            ctx.fix_perms(),
                            || unlink_at(parent.fd, name, libc::AT_REMOVEDIR),
                            || parent.make_writable(),
                        )
                        .is_ok();
                    let path = parent.child_path(name.to_bytes());
                    if removed {
                        ctx.dir_removed(|| path);
                    } else {
                        ctx.error_at("cannot remove", &path, e);
                        parent.fail();
                    }
                }
            }
        }

        /// List a directory: files go straight into the batch, subdirectory
        /// names are returned for the walk. Reads through a duplicate
        /// descriptor so the `DIR` buffer is freed as soon as listing ends
        /// while the node keeps its own descriptor for the workers'
        /// `unlinkat`.
        fn read_dir(&mut self, node: &Arc<DirNode<'c>>) -> Vec<CString> {
            let ctx = self.ctx;
            let mut subdirs = Vec::new();
            // SAFETY: node.fd is open.
            let rfd = unsafe { libc::fcntl(node.fd, libc::F_DUPFD_CLOEXEC, 0) };
            if rfd < 0 {
                ctx.error_at("cannot open directory", &node.path(), errno());
                node.fail();
                return subdirs;
            }
            // SAFETY: rfd is a fresh descriptor; on success the DIR owns it.
            let dirp = unsafe { libc::fdopendir(rfd) };
            if dirp.is_null() {
                let e = errno();
                // SAFETY: fdopendir failed, so rfd is still ours.
                unsafe { libc::close(rfd) };
                ctx.error_at("cannot open directory", &node.path(), e);
                node.fail();
                return subdirs;
            }

            loop {
                clear_errno();
                // SAFETY: dirp is a valid open DIR.
                let ent = unsafe { libc::readdir(dirp) };
                if ent.is_null() {
                    let e = errno();
                    if e != 0 {
                        ctx.error_at("cannot read directory", &node.path(), e);
                        node.fail();
                    }
                    break;
                }
                // SAFETY: readdir returned a valid entry with a NUL-terminated
                // name; it stays valid until the next readdir call.
                let (name, d_type, d_ino) = unsafe {
                    (
                        CStr::from_ptr((*ent).d_name.as_ptr()),
                        (*ent).d_type,
                        (*ent).d_ino,
                    )
                };
                let bytes = name.to_bytes();
                if bytes == b"." || bytes == b".." {
                    continue;
                }
                let is_dir = match d_type {
                    libc::DT_DIR => true,
                    // XFS, NFS, overlayfs and friends may not fill in d_type.
                    libc::DT_UNKNOWN => lstat_at(node.fd, name).is_some_and(|st| is_dir_mode(&st)),
                    _ => false,
                };
                if is_dir {
                    subdirs.push(name.to_owned());
                } else if LIST_THEN_UNLINK {
                    self.listed.push((d_ino, self.listed_names.len() as u32));
                    self.listed_names.extend_from_slice(bytes);
                    self.listed_names.push(0);
                } else {
                    self.add_file(Some(node), bytes);
                }
            }
            // SAFETY: closes the DIR and rfd, once.
            unsafe { libc::closedir(dirp) };

            if LIST_THEN_UNLINK && !self.listed.is_empty() {
                let mut listed = std::mem::take(&mut self.listed);
                let names = std::mem::take(&mut self.listed_names);
                if listed.len() >= INODE_SORT_MIN {
                    listed.sort_unstable_by_key(|&(ino, _)| ino);
                }
                for &(_, off) in &listed {
                    let tail = &names[off as usize..];
                    if let Ok(name) = CStr::from_bytes_until_nul(tail) {
                        self.add_file(Some(node), name.to_bytes());
                    }
                }
                listed.clear();
                self.listed = listed;
                self.listed_names = names;
                self.listed_names.clear();
            }
            subdirs
        }
    }

    // -----------------------------------------------------------------------
    // Running a removal
    // -----------------------------------------------------------------------

    fn run(paths: &[PathBuf], opts: &Options) -> Outcome {
        let budget = open_dir_budget();
        let mut ctx = Ctx::new(*opts);

        LAST_PEAK_OPEN.with(|c| c.set(0));
        let all: Vec<usize> = (0..paths.len()).collect();
        run_pass(&ctx, paths, &all, budget);

        let mut retry = std::mem::take(&mut *lock(&ctx.retry));
        retry.sort_unstable();
        retry.dedup();
        LAST_RETRIES.with(|c| c.set(retry.len()));
        if !retry.is_empty() {
            ctx.allow_retry = false;
            run_pass(&ctx, paths, &retry, budget);
        }

        let mut stats = std::mem::take(&mut *lock(&ctx.stats));
        stats.dirs_deleted += ctx.dirs_removed.load(Ordering::Relaxed);
        let mut errors = std::mem::take(&mut *lock(&ctx.errors));
        if !retry.is_empty() {
            // The second pass may repeat an error from the first.
            let mut seen = HashSet::new();
            errors.retain(|e| seen.insert(e.clone()));
        }
        Outcome { stats, errors }
    }

    fn run_pass(ctx: &Ctx, paths: &[PathBuf], which: &[usize], budget: usize) {
        let queue = Queue::new();
        std::thread::scope(|scope| {
            let mut walker = Walker {
                ctx,
                queue: &queue,
                scope,
                budget,
                peak: 0,
                batch: Batch::new(),
                pooled: false,
                stats: DeleteStats::new(),
                out: Vec::new(),
                listed: Vec::new(),
                listed_names: Vec::new(),
            };
            for &i in which {
                walker.operand(i, &paths[i]);
            }
            walker.finish();
        });
        // Every worker is joined and every DirNode dropped at this point.
        ctx.flush_out();
    }

    // -----------------------------------------------------------------------
    // Interactive mode
    // -----------------------------------------------------------------------

    struct Interactive<'q> {
        opts: Options,
        ask: &'q mut dyn FnMut(&str) -> bool,
        out: Outcome,
        links: HashSet<(u64, u64)>,
        /// st_dev of the operand being walked (for `one_file_system`).
        dev: u64,
        /// See `__set_pretend_other_device`.
        pretend_xdev: bool,
    }

    struct IFrame {
        path: PathBuf,
        entries: Vec<PathBuf>,
        next: usize,
        /// Something inside was kept, so this directory stays too.
        kept: bool,
    }

    enum EntryResult {
        Removed,
        Kept,
        Descend(IFrame),
    }

    /// -W: add u+rwx to the directory at `path`.
    fn add_rwx_path(path: &Path, follow: bool) -> bool {
        match CString::new(path.as_os_str().as_bytes()) {
            Ok(c) => add_rwx_at(libc::AT_FDCWD, &c, follow),
            Err(_) => false,
        }
    }

    fn parent_path(path: &Path) -> &Path {
        match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        }
    }

    fn read_entries(path: &Path) -> io::Result<Vec<PathBuf>> {
        fs::read_dir(path)?
            .map(|e| e.map(|e| path.join(e.file_name())))
            .collect()
    }

    impl Interactive<'_> {
        fn error(&mut self, verb: &str, path: &Path, e: &io::Error) {
            self.out.errors.push(format!(
                "{verb} '{}': {}",
                path.display(),
                super::io_err_text(e)
            ));
        }

        fn missing_ok(&self, e: &io::Error) -> bool {
            self.opts.force && e.kind() == io::ErrorKind::NotFound
        }

        /// -W applies to this error.
        fn fixable(&self, e: &io::Error) -> bool {
            self.opts.force_writable
                && !self.opts.dry_run
                && e.kind() == io::ErrorKind::PermissionDenied
        }

        fn verbose(&self, dir: bool, path: &Path) {
            if !self.opts.verbose {
                return;
            }
            let what: &[u8] = match (self.opts.dry_run, dir) {
                (false, false) => b"removed '",
                (false, true) => b"removed directory '",
                (true, false) => b"would remove '",
                (true, true) => b"would remove directory '",
            };
            let mut line = what.to_vec();
            line.extend_from_slice(path.as_os_str().as_bytes());
            line.extend_from_slice(b"'\n");
            write_stdout(&line);
        }

        fn size(&mut self, meta: &fs::Metadata) -> u64 {
            if !self.opts.track_size {
                return 0;
            }
            let key = (meta.dev(), meta.ino());
            if meta.nlink() > 1 || self.links.contains(&key) {
                if self.links.insert(key) {
                    meta.len()
                } else {
                    0
                }
            } else {
                meta.len()
            }
        }

        /// Run a removal; with -W, make the containing directory writable
        /// after EACCES/EPERM and try once more.
        fn removing(&self, path: &Path, op: impl Fn(&Path) -> io::Result<()>) -> io::Result<()> {
            match op(path) {
                Err(e) if self.fixable(&e) && add_rwx_path(parent_path(path), true) => op(path),
                r => r,
            }
        }

        fn list(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
            match read_entries(path) {
                Err(e) if self.fixable(&e) && add_rwx_path(path, false) => read_entries(path),
                r => r,
            }
        }

        fn dir_removed(&mut self, path: &Path) {
            self.out.stats.dirs_deleted += 1;
            if let Some(p) = self.opts.progress {
                p.add(0, 1, 0);
            }
            self.verbose(true, path);
        }

        fn operand(&mut self, path: &Path) {
            let meta = match fs::symlink_metadata(path) {
                Ok(m) => m,
                Err(e) => {
                    // GNU -f is also silent for `rm -f file/x` (ENOTDIR).
                    let missing = e.kind() == io::ErrorKind::NotFound
                        || e.raw_os_error() == Some(libc::ENOTDIR);
                    if !(self.opts.force && missing) {
                        self.error("cannot remove", path, &e);
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
                    self.out.errors.push(format!(
                        "cannot remove '{}': {}",
                        path.display(),
                        strerror(libc::EISDIR)
                    ));
                } else if (self.ask)(&format!("remove directory '{}'?", path.display())) {
                    self.rmdir(path, true);
                }
                return;
            }

            self.dev = if self.pretend_xdev {
                !meta.dev()
            } else {
                meta.dev()
            };
            let mut stack: Vec<IFrame> = Vec::new();
            match self.enter(path) {
                EntryResult::Descend(frame) => stack.push(frame),
                EntryResult::Removed | EntryResult::Kept => return,
            }
            while let Some(top) = stack.last_mut() {
                if top.next < top.entries.len() {
                    let child = std::mem::take(&mut top.entries[top.next]);
                    top.next += 1;
                    match self.entry(&child) {
                        EntryResult::Removed => {}
                        EntryResult::Kept => {
                            if let Some(t) = stack.last_mut() {
                                t.kept = true;
                            }
                        }
                        EntryResult::Descend(frame) => stack.push(frame),
                    }
                    continue;
                }
                let Some(done) = stack.pop() else { break };
                let removed = !done.kept
                    && (self.ask)(&format!("remove directory '{}'?", done.path.display()))
                    && self.rmdir(&done.path, false);
                if !removed {
                    if let Some(parent) = stack.last_mut() {
                        parent.kept = true;
                    }
                }
            }
        }

        /// List a directory and, unless it is empty, ask to descend into it.
        /// An empty directory goes straight to `remove directory` (GNU).
        fn enter(&mut self, path: &Path) -> EntryResult {
            let descend = |p: &Path| format!("descend into directory '{}'?", p.display());
            match self.list(path) {
                Ok(entries) => {
                    if !entries.is_empty() && !(self.ask)(&descend(path)) {
                        return EntryResult::Kept;
                    }
                    EntryResult::Descend(IFrame {
                        path: path.to_path_buf(),
                        entries,
                        next: 0,
                        kept: false,
                    })
                }
                Err(e) if self.missing_ok(&e) => EntryResult::Removed,
                Err(e) => {
                    // Unreadable, so possibly non-empty: ask as for a full
                    // directory, then offer to remove it anyway.
                    if !(self.ask)(&descend(path))
                        || !(self.ask)(&format!("remove directory '{}'?", path.display()))
                    {
                        return EntryResult::Kept;
                    }
                    if !self.opts.dry_run && self.removing(path, |p| fs::remove_dir(p)).is_ok() {
                        self.dir_removed(path);
                        return EntryResult::Removed;
                    }
                    self.error("cannot remove", path, &e);
                    EntryResult::Kept
                }
            }
        }

        fn entry(&mut self, path: &Path) -> EntryResult {
            // -W: the directory may be readable but not searchable.
            let meta = match fs::symlink_metadata(path) {
                Err(e) if self.fixable(&e) && add_rwx_path(parent_path(path), false) => {
                    fs::symlink_metadata(path)
                }
                r => r,
            };
            let meta = match meta {
                Ok(m) => m,
                Err(e) => {
                    if self.missing_ok(&e) {
                        return EntryResult::Removed;
                    }
                    self.error("cannot remove", path, &e);
                    return EntryResult::Kept;
                }
            };
            if meta.is_dir() {
                if self.opts.one_file_system && meta.dev() != self.dev {
                    self.out.errors.push(format!(
                        "skipping '{}', since it's on a different device",
                        path.display()
                    ));
                    return EntryResult::Kept;
                }
                return self.enter(path);
            }
            if self.file(path, &meta) {
                EntryResult::Removed
            } else {
                EntryResult::Kept
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
                self.removing(path, |p| fs::remove_file(p))
            };
            match res {
                Ok(()) => {
                    let size = self.size(meta);
                    self.out.stats.files_deleted += 1;
                    self.out.stats.total_size += size;
                    if let Some(p) = self.opts.progress {
                        p.add(1, 0, size);
                    }
                    self.verbose(false, path);
                    true
                }
                Err(e) => {
                    if self.missing_ok(&e) {
                        return true;
                    }
                    self.error("cannot remove", path, &e);
                    false
                }
            }
        }

        /// rmdir `path`. In a dry run, `check_empty` decides whether the
        /// directory must really be empty (`-d`) or is known to be about to
        /// become empty (everything inside was removed).
        fn rmdir(&mut self, path: &Path, check_empty: bool) -> bool {
            let res = if !self.opts.dry_run {
                self.removing(path, |p| fs::remove_dir(p))
            } else if check_empty {
                match fs::read_dir(path).map(|mut rd| rd.next().is_none()) {
                    Ok(true) => Ok(()),
                    Ok(false) => Err(io::Error::from_raw_os_error(libc::ENOTEMPTY)),
                    Err(e) => Err(e),
                }
            } else {
                Ok(())
            };
            match res {
                Ok(()) => {
                    self.dir_removed(path);
                    true
                }
                Err(e) => {
                    if self.missing_ok(&e) {
                        return true;
                    }
                    self.error("cannot remove", path, &e);
                    false
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parent_dir_of_bare_and_nested_names() {
            assert_eq!(parent_dir(b"f").as_bytes(), b".");
            assert_eq!(parent_dir(b"a/b/c").as_bytes(), b"a/b");
            assert_eq!(parent_dir(b"/x").as_bytes(), b"/");
        }

        #[test]
        fn retry_runs_again_only_after_a_successful_fix() {
            use std::cell::Cell;
            let calls = Cell::new(0);
            let op = || {
                calls.set(calls.get() + 1);
                if calls.get() == 1 {
                    Err(libc::EACCES)
                } else {
                    Ok(())
                }
            };
            assert_eq!(with_retry(true, op, || true), Ok(()));
            assert_eq!(calls.get(), 2);

            calls.set(0);
            assert_eq!(with_retry(true, op, || false), Err(libc::EACCES));
            assert_eq!(calls.get(), 1);

            calls.set(0);
            assert_eq!(with_retry(false, op, || true), Err(libc::EACCES));
            assert_eq!(calls.get(), 1);

            // Other errors are never retried.
            let enoent = || Err::<(), _>(libc::ENOENT);
            assert_eq!(
                with_retry(true, enoent, || panic!("fix")),
                Err(libc::ENOENT)
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Legacy entry points (portable; built on `remove_all`)
// ---------------------------------------------------------------------------

fn into_result(outcome: Outcome) -> Result<DeleteStats, String> {
    if outcome.errors.is_empty() {
        Ok(outcome.stats)
    } else {
        Err(outcome.errors.join("\n"))
    }
}

/// Ask on stderr, read the answer from stdin. Anything starting with `y` is
/// yes.
fn ask_stdin(question: &str) -> bool {
    use std::io::Write;
    eprint!("rmx: {question} ");
    let _ = io::stderr().flush();
    let mut answer = String::new();
    match io::stdin().read_line(&mut answer) {
        Ok(_) => matches!(answer.trim_start().chars().next(), Some('y' | 'Y')),
        Err(_) => false,
    }
}

/// Remove a single non-directory.
pub fn remove_file(path: &Path, verbose: bool, force: bool) -> Result<DeleteStats, String> {
    let opts = Options {
        recursive: false,
        dir: false,
        ..Options::new(verbose, force)
    };
    into_result(remove_all(&[path.to_path_buf()], &opts))
}

/// Recursively remove a directory's *contents*. The directory itself is left
/// in place for the caller to remove. Errors if `path` is not a directory.
pub fn remove_directory_recursive_with(path: &Path, opts: &Options) -> Result<DeleteStats, String> {
    let cannot = |why: String| Err(format!("cannot remove '{}': {why}", path.display()));
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => {}
        Ok(_) => return cannot("Not a directory".to_string()),
        Err(e) if opts.force && e.kind() == io::ErrorKind::NotFound => {
            return Ok(DeleteStats::new())
        }
        Err(e) => return cannot(io_err_text(&e)),
    }
    let entries = std::fs::read_dir(path).and_then(|rd| {
        rd.map(|e| e.map(|e| path.join(e.file_name())))
            .collect::<io::Result<Vec<_>>>()
    });
    let entries = match entries {
        Ok(entries) => entries,
        Err(e) => return cannot(io_err_text(&e)),
    };
    let opts = Options {
        recursive: true,
        ..*opts
    };
    into_result(remove_all(&entries, &opts))
}

/// rmdir an empty directory.
pub fn remove_empty_directory(
    path: &Path,
    verbose: bool,
    force: bool,
) -> Result<DeleteStats, String> {
    let opts = Options {
        recursive: false,
        dir: true,
        ..Options::new(verbose, force)
    };
    into_result(remove_all(&[path.to_path_buf()], &opts))
}

/// Ask on the terminal, then remove a single non-directory.
pub fn remove_file_interactive(
    path: &Path,
    verbose: bool,
    force: bool,
) -> Result<DeleteStats, String> {
    let opts = Options {
        recursive: false,
        dir: false,
        ..Options::new(verbose, force)
    };
    into_result(remove_all_interactive(
        &[path.to_path_buf()],
        &opts,
        &mut ask_stdin,
    ))
}

/// Interactively remove a directory tree, including the directory itself.
pub fn remove_directory_interactive_with(
    path: &Path,
    opts: &Options,
) -> Result<DeleteStats, String> {
    let opts = Options {
        recursive: true,
        ..*opts
    };
    into_result(remove_all_interactive(
        &[path.to_path_buf()],
        &opts,
        &mut ask_stdin,
    ))
}

// The portable engine is what Windows runs; its unit tests also run on unix.
#[cfg(any(not(unix), test))]
mod portable;
#[cfg(not(unix))]
pub use portable::{remove_all, remove_all_interactive};
