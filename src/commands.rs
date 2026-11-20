use std::fs;
use std::io::{BufRead, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::args::RmxArgs;
use crate::style::{self, Stream};
use crate::util::bytes_to_readable;
use rmx_lib::{remove_all, remove_all_interactive, DeleteStats, Options};

fn report_error(msg: &str) {
    eprintln!(
        "{}",
        style::paint(Stream::Stderr, style::RED, &format!("rmx: {msg}"))
    );
}

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

    let opts = Options {
        recursive: args.recursive,
        dir: args.dir,
        force: args.force,
        verbose: args.verbose,
        track_size: !args.quiet,
        jobs: args.jobs,
        ..Options::default()
    };

    let outcome = if paths.is_empty() {
        Default::default()
    } else if args.interactive {
        remove_all_interactive(&paths, &opts, &mut ask)
    } else {
        remove_all(&paths, &opts)
    };

    for e in &outcome.errors {
        report_error(e);
    }
    failed |= !outcome.is_ok();

    if !args.quiet {
        print_summary(&outcome.stats, start_time.elapsed());
    }

    i32::from(failed)
}

/// Safety checks every operand must pass before the engine sees it.
///
/// Returns the path to hand to the engine (trailing slashes stripped).
fn check_operand(path: &Path, args: &RmxArgs) -> Result<PathBuf, String> {
    let shown = path.display();
    let bytes = path.as_os_str().as_bytes();

    // Strip trailing slashes, but keep a bare "/" (or "//") as "/".
    let mut end = bytes.len();
    while end > 1 && bytes[end - 1] == b'/' {
        end -= 1;
    }
    let had_trailing_slash = end < bytes.len();
    let stripped = Path::new(std::ffi::OsStr::from_bytes(&bytes[..end]));

    // POSIX: refuse any operand whose last component is "." or "..".
    let last = match stripped
        .as_os_str()
        .as_bytes()
        .rsplit(|b| *b == b'/')
        .next()
    {
        Some(l) => l,
        None => b"",
    };
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

    if had_trailing_slash && stripped != Path::new("/") {
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
fn is_root(path: &Path) -> bool {
    let (Ok(target), Ok(root)) = (fs::symlink_metadata(path), fs::metadata("/")) else {
        return false;
    };
    target.is_dir() && target.dev() == root.dev() && target.ino() == root.ino()
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

fn print_summary(stats: &DeleteStats, elapsed_time: std::time::Duration) {
    if stats.files_deleted == 0 && stats.dirs_deleted == 0 {
        return;
    }

    let lines = format!(
        "✓ Deleted: {} files, {} directories\n\
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
  • Verified upgrades: SHA-256 verified against the release's SHA256SUMS
  • Linux and macOS

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
