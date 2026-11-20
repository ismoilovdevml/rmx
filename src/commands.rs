use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::args::RmxArgs;
use crate::style::{self, Stream};
use crate::util::bytes_to_readable;
use rmx_lib::{remove_all, remove_all_interactive, DeleteStats, Options, Progress};

fn report_error(msg: &str) {
    eprintln!(
        "{}",
        style::paint(Stream::Stderr, style::RED, &format!("rmx: {msg}"))
    );
}

static PROGRESS: Progress = Progress {
    files: AtomicU64::new(0),
    dirs: AtomicU64::new(0),
    bytes: AtomicU64::new(0),
};

/// Run a removal and return the process exit code (0 = everything removed).
pub fn execute_removal(args: &RmxArgs) -> i32 {
    let start_time = Instant::now();
    let mut failed = false;

    let mut paths = Vec::with_capacity(args.paths.len());
    for path in &args.paths {
        match check_operand(path, args) {
            Ok(p) => paths.push(p),
            Err(msg) => {
                report_error(&msg);
                failed = true;
            }
        }
    }

    // --async: move whole directory operands out of the way now and delete
    // them in a detached child; everything else is handled right here.
    let mut moved = 0usize;
    if args.background && args.recursive {
        let (trash, rest) = move_to_trash(paths);
        paths = rest;
        moved = trash.len();
        if !trash.is_empty() {
            if let Err(e) = spawn_background(&trash) {
                report_error(&format!("cannot start background deletion: {e}"));
                // Fall back to deleting synchronously.
                paths.extend(trash);
                moved = 0;
            }
        }
    }

    // A live progress line only when someone is watching and nothing else
    // is being printed (verbose lines or prompts would interleave with it).
    let show_progress = !args.quiet
        && !args.verbose
        && !args.interactive
        && style::progress_allowed()
        && std::io::stderr().is_terminal();

    let opts = Options {
        recursive: args.recursive,
        dir: args.dir,
        force: args.force,
        verbose: args.verbose,
        track_size: !args.quiet,
        jobs: args.jobs,
        dry_run: args.dry_run,
        one_file_system: args.one_fs,
        force_writable: args.force_writable,
        progress: show_progress.then_some(&PROGRESS),
    };

    let outcome = if paths.is_empty() {
        Default::default()
    } else if args.interactive {
        remove_all_interactive(&paths, &opts, &mut ask)
    } else if show_progress {
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let dry_run = args.dry_run;
        let painter = std::thread::spawn(move || progress_loop(done_rx, dry_run));
        let outcome = remove_all(&paths, &opts);
        drop(done_tx);
        let _ = painter.join();
        outcome
    } else {
        remove_all(&paths, &opts)
    };

    for e in &outcome.errors {
        report_error(e);
    }
    failed |= !outcome.is_ok();

    if !args.quiet {
        print_summary(&outcome.stats, start_time.elapsed(), args.dry_run);
        if moved > 0 {
            println!(
                "{}",
                style::paint(
                    Stream::Stdout,
                    style::GREEN,
                    &format!("✓ Deleting {moved} more in the background")
                )
            );
        }
    }

    i32::from(failed)
}

/// Repaint a one-line counter on stderr every 100 ms, starting only once the
/// job has run for 300 ms so short deletions print nothing extra.
fn progress_loop(done: mpsc::Receiver<()>, dry_run: bool) {
    if done.recv_timeout(Duration::from_millis(300)) != Err(mpsc::RecvTimeoutError::Timeout) {
        return;
    }
    let verb = if dry_run { "would remove" } else { "removed" };
    let mut err = std::io::stderr();
    loop {
        let _ = write!(
            err,
            "\r\x1b[Krmx: {verb} {} files, {} directories, {}",
            PROGRESS.files.load(Ordering::Relaxed),
            PROGRESS.dirs.load(Ordering::Relaxed),
            bytes_to_readable(PROGRESS.bytes.load(Ordering::Relaxed)),
        );
        let _ = err.flush();
        if done.recv_timeout(Duration::from_millis(100)) != Err(mpsc::RecvTimeoutError::Timeout) {
            break;
        }
    }
    let _ = write!(err, "\r\x1b[K");
    let _ = err.flush();
}

/// Rename every real directory operand to a hidden sibling. A rename within
/// one directory is atomic and instant, so the name is free again at once.
/// Returns (moved trash paths, operands still to delete here).
fn move_to_trash(paths: Vec<PathBuf>) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut trash = Vec::new();
    let mut rest = Vec::new();
    for (n, path) in paths.into_iter().enumerate() {
        let is_dir = fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir());
        if !is_dir {
            rest.push(path);
            continue;
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let name = format!(".rmx-trash-{}-{n}-{nanos:x}", std::process::id());
        let target = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.join(name),
            _ => PathBuf::from(name),
        };
        match fs::rename(&path, &target) {
            Ok(()) => trash.push(target),
            Err(_) => rest.push(path), // e.g. no write permission: do it now
        }
    }
    (trash, rest)
}

/// Start `rmx -rfq -- <trash...>` detached from this terminal session.
fn spawn_background(trash: &[PathBuf]) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("-rfq")
        .arg("--")
        .args(trash)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is async-signal-safe and touches no Rust state.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    cmd.spawn().map(|_| ())
}

fn is_sep(b: u8) -> bool {
    b == b'/' || (cfg!(windows) && b == b'\\')
}

/// Safety checks every operand must pass before the engine sees it.
///
/// Returns the path to hand to the engine (trailing slashes stripped).
fn check_operand(path: &Path, args: &RmxArgs) -> Result<PathBuf, String> {
    let shown = path.display();
    let bytes = path.as_os_str().as_encoded_bytes();

    // Strip trailing separators, but keep a bare "/" (or "//") as "/".
    let mut end = bytes.len();
    while end > 1 && is_sep(bytes[end - 1]) {
        end -= 1;
    }
    let had_trailing_slash = end < bytes.len();
    // SAFETY: we only cut at an ASCII separator, which is a valid boundary
    // in the platform's OsStr encoding.
    let stripped = Path::new(unsafe { OsStr::from_encoded_bytes_unchecked(&bytes[..end]) });

    // POSIX: refuse any operand whose last component is "." or "..".
    let last = bytes[..end].rsplit(|b| is_sep(*b)).next().unwrap_or(b"");
    if last == b"." || last == b".." {
        return Err(format!(
            "refusing to remove '.' or '..' directory: skipping '{shown}'"
        ));
    }

    if args.recursive && args.preserve_root && is_root(stripped) {
        return Err(format!(
            "it is dangerous to operate recursively on '{shown}'\n\
             rmx: use --no-preserve-root to override this failsafe"
        ));
    }

    if had_trailing_slash && end > 1 {
        if let Ok(meta) = fs::symlink_metadata(stripped) {
            if meta.file_type().is_symlink() {
                // `link/` would mean "the directory the link points to".
                // Refuse rather than delete data outside the named path.
                return Err(format!(
                    "cannot remove '{shown}': Not a directory \
                     (it is a symlink; drop the trailing '/' to remove the link itself)"
                ));
            }
            if !meta.is_dir() {
                return Err(format!("cannot remove '{shown}': Not a directory"));
            }
        }
    }

    Ok(stripped.to_path_buf())
}

/// Same directory as `/`? Compared by device and inode without following the
/// operand if it is a symlink, so `rmx -rf link-to-root` just unlinks the link.
#[cfg(unix)]
fn is_root(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let (Ok(target), Ok(root)) = (fs::symlink_metadata(path), fs::metadata("/")) else {
        return false;
    };
    target.is_dir() && target.dev() == root.dev() && target.ino() == root.ino()
}

/// A drive or share root such as `C:\`: a real directory with no parent.
#[cfg(not(unix))]
fn is_root(path: &Path) -> bool {
    let is_real_dir = fs::symlink_metadata(path).is_ok_and(|m| m.is_dir());
    is_real_dir && fs::canonicalize(path).is_ok_and(|p| p.parent().is_none())
}

/// Interactive prompt on stderr, answer from stdin.
fn ask(question: &str) -> bool {
    eprint!("rmx: {question} ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => false,
        Ok(_) => {
            let answer = line.trim().to_ascii_lowercase();
            answer == "y" || answer == "yes"
        }
    }
}

fn print_summary(stats: &DeleteStats, elapsed_time: Duration, dry_run: bool) {
    if stats.files_deleted == 0 && stats.dirs_deleted == 0 {
        return;
    }

    let verb = if dry_run { "Would delete" } else { "Deleted" };
    let lines = format!(
        "✓ {verb}: {} files, {} directories\n\
         ✓ Total size: {}\n\
         ✓ Time taken: {:.2?}",
        stats.files_deleted,
        stats.dirs_deleted,
        bytes_to_readable(stats.total_size),
        elapsed_time,
    );
    // One locked write instead of three separate println! calls.
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = writeln!(
        out,
        "{}",
        style::paint(Stream::Stdout, style::GREEN, &lines)
    );
}

pub fn print_version() {
    println!(
        "{}",
        style::paint(
            Stream::Stdout,
            style::GREEN,
            &format!("rmx v{}", env!("CARGO_PKG_VERSION"))
        )
    );
}

fn heading(text: &str) -> String {
    style::paint(Stream::Stdout, style::CYAN, text)
}

pub fn print_help() {
    println!(
        "{title} - a faster rm

{usage}
    rmx [OPTIONS] [--] <FILE|DIRECTORY>...
    rmx <COMMAND>

{options}
    -r, -R, --recursive     Remove directories and their contents recursively
    -f, --force             Ignore nonexistent files, never prompt
    -i, --interactive       Prompt before every removal
    -v, --verbose           Explain what is being done
    -d, --dir               Remove empty directories
    -q, --quiet             Suppress the summary and skip byte accounting
    -j, --jobs <N>          Worker threads for unlinking (default: min(cores, 4))
    -n, --dry-run           Show what would be removed; remove nothing
    -x, --one-file-system   Skip directories on a different file system
    -W, --force-writable    Make read-only directories writable so their
                            contents can go (Go module cache, Bazel, ...)
        --async             Move directory trees aside instantly and delete
                            them in a background process
        --no-preserve-root  Do not treat '/' specially
        --preserve-root     Refuse to operate recursively on '/' (default)
    --                      Treat every following argument as a path
    -V, --version           Show version
    -h, --help              Show this help message

    The last of -f / -i wins. -f only ignores missing files; every other
    error (permission denied, read-only file system, ...) is still reported
    and makes rmx exit 1.

{commands}
    about                   Show program information
    dev                     Show developer information
    upgrade                 Upgrade to the latest release (SHA-256 verified)
    check-update            Check if a new version is available
    version, help           Same as --version, --help

    Commands are only recognised as the sole argument. To remove a file
    named like a command, use 'rmx -- version' or 'rmx ./version'.

{examples}
    rmx file.txt                    Remove a single file
    rmx -rf node_modules/           Remove a directory tree
    rmx -rfv build/                 Show every removal
    rmx -rfq cache/                 Silent, for scripts
    rmx -i file.txt                 Ask first
    rmx -d empty_dir/               Remove a directory only if empty
    rmx -- -weird-name.txt          Remove a file whose name starts with '-'
    rmx -rn node_modules/           Dry run: count what would go
    rmx -rf --async node_modules/   Return at once, delete in the background
    rmx -rfW ~/go/pkg/mod           Remove a read-only module cache

{warning}
    rmx permanently deletes files. There is no trash. Check the path first.",
        title = style::paint(
            Stream::Stdout,
            style::GREEN,
            &format!("rmx v{}", env!("CARGO_PKG_VERSION"))
        ),
        usage = heading("USAGE:"),
        options = heading("OPTIONS:"),
        commands = heading("COMMANDS:"),
        examples = heading("EXAMPLES:"),
        warning = style::paint(Stream::Stdout, style::RED, "WARNING:"),
    );
}

pub fn print_about() {
    println!(
        "{title}

A drop-in for the rm flags you already use (-r -f -i -v -d), written in Rust.
One thread walks the tree with openat/readdir while a small pool of workers
unlinks files relative to their directory's file descriptor. Symlinks are
never followed.

{features}
  • rm-compatible flags, '--', --preserve-root, '.'/'..' refusal
  • -f ignores only missing files; real errors are reported
  • Parallel unlinking capped at 4 workers (the measured optimum)
  • Byte totals with hard links counted once
  • --dry-run, --async, --one-file-system, --force-writable
  • Live progress line for long deletions (terminal only)
  • Verified upgrades: SHA-256 verified against the release's SHA256SUMS
  • Linux, macOS and Windows

Benchmarks: https://github.com/ismoilovdevml/rmx#performance",
        title = style::paint(Stream::Stdout, style::GREEN, "rmx - a faster rm"),
        features = heading("Features:"),
    );
}

pub fn print_dev() {
    println!(
        "{title}

{author}       Otabek Ismoilov
{email}        ismoilovdev@gmail.com
{github}       @ismoilovdevml
{repo}   https://github.com/ismoilovdevml/rmx
{license}      MIT
{version}      v{v}",
        title = style::paint(Stream::Stdout, style::GREEN, "Developer Information"),
        author = heading("Author:"),
        email = heading("Email:"),
        github = heading("GitHub:"),
        repo = heading("Repository:"),
        license = heading("License:"),
        version = heading("Version:"),
        v = env!("CARGO_PKG_VERSION"),
    );
}
