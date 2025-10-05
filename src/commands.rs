use std::path::Path;
use std::time::Instant;

use termion::{color, style};

use crate::args::RmxArgs;
use crate::util::bytes_to_readable;
use rmx_lib::{
    remove_directory_interactive, remove_directory_recursive, remove_empty_directory, remove_file,
    remove_file_interactive, DeleteStats,
};

pub fn execute_removal(args: &RmxArgs) {
    let start_time = Instant::now();
    let mut total_stats = DeleteStats::new();

    for path_str in &args.paths {
        let path = Path::new(path_str);

        // Check if path exists
        if !path.exists() {
            if !args.force {
                eprintln!(
                    "{}rmx: cannot remove '{}': No such file or directory{}",
                    color::Fg(color::LightRed),
                    path_str,
                    style::Reset
                );
                std::process::exit(1);
            }
            continue;
        }

        // Handle based on file type and flags
        let result = if path.is_dir() {
            handle_directory(path, args)
        } else {
            handle_file(path, args)
        };

        match result {
            Ok(stats) => total_stats.merge(stats),
            Err(e) => {
                eprintln!("{}{}{}", color::Fg(color::LightRed), e, style::Reset);
                if !args.force {
                    std::process::exit(1);
                }
            }
        }
    }

    let elapsed_time = start_time.elapsed();

    // Print summary
    print_summary(&total_stats, elapsed_time, args.verbose);
}

fn handle_file(path: &Path, args: &RmxArgs) -> Result<DeleteStats, String> {
    if args.interactive {
        remove_file_interactive(path, args.verbose, args.force)
    } else {
        remove_file(path, args.verbose, args.force)
    }
}

fn handle_directory(path: &Path, args: &RmxArgs) -> Result<DeleteStats, String> {
    if args.recursive {
        // Recursive deletion
        if args.interactive {
            remove_directory_interactive(path, args.verbose, args.force)
        } else {
            let mut stats = remove_directory_recursive(path, args.verbose, args.force)?;
            // Remove the directory itself
            match std::fs::remove_dir(path) {
                Ok(_) => {
                    stats.dirs_deleted += 1;
                    if args.verbose {
                        println!("removed directory '{}'", path.display());
                    }
                    Ok(stats)
                }
                Err(e) => {
                    if args.force {
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
    } else if args.dir {
        // Remove empty directory
        remove_empty_directory(path, args.verbose, args.force)
    } else {
        Err(format!(
            "rmx: cannot remove '{}': Is a directory (use -r to remove directories)",
            path.display()
        ))
    }
}

fn print_summary(stats: &DeleteStats, elapsed_time: std::time::Duration, verbose: bool) {
    // Always show summary if files were deleted (unless quiet mode is active in future)
    if stats.files_deleted > 0 || stats.dirs_deleted > 0 {
        if !verbose {
            // Compact summary for non-verbose mode
            println!(
                "{}✓ Deleted: {} files, {} directories{}",
                color::Fg(color::LightGreen),
                stats.files_deleted,
                stats.dirs_deleted,
                style::Reset
            );
            println!(
                "{}✓ Total size: {}{}",
                color::Fg(color::LightGreen),
                bytes_to_readable(stats.total_size),
                style::Reset
            );
            println!(
                "{}✓ Time taken: {:.2?}{}",
                color::Fg(color::LightGreen),
                elapsed_time,
                style::Reset
            );
        } else {
            // Verbose mode shows individual file deletions + summary
            println!(
                "{}✓ Deleted: {} files, {} directories{}",
                color::Fg(color::LightGreen),
                stats.files_deleted,
                stats.dirs_deleted,
                style::Reset
            );
            println!(
                "{}✓ Total size: {}{}",
                color::Fg(color::LightGreen),
                bytes_to_readable(stats.total_size),
                style::Reset
            );
            println!(
                "{}✓ Time taken: {:.2?}{}",
                color::Fg(color::LightGreen),
                elapsed_time,
                style::Reset
            );
        }
    }
}

pub fn print_version() {
    println!(
        "{}rmx v{}{}",
        color::Fg(color::LightGreen),
        env!("CARGO_PKG_VERSION"),
        style::Reset
    );
}

pub fn print_help() {
    println!(
        "{}rmx v{}{} - Blazing fast alternative to rm command

{}USAGE:{}
    rmx [OPTIONS] <FILE|DIRECTORY>...

{}OPTIONS:{}
    -r, -R, --recursive     Remove directories and their contents recursively
    -f, --force             Ignore nonexistent files, never prompt
    -i, --interactive       Prompt before every removal
    -v, --verbose           Explain what is being done
    -d, --dir               Remove empty directories

    --version               Show version
    --help                  Show this help message

{}COMMANDS:{}
    about                   Show program information
    dev                     Show developer information
    upgrade                 Upgrade to the latest version
    check-update            Check if a new version is available

{}EXAMPLES:{}
    rmx file.txt                    Remove a single file
    rmx file1.txt file2.txt         Remove multiple files
    rmx -r directory/               Remove directory recursively
    rmx -rf /tmp/test/              Force remove directory
    rmx -i file.txt                 Interactive removal
    rmx -v -r build/                Verbose recursive removal
    rmx -d empty_dir/               Remove empty directory
    rmx upgrade                     Upgrade to latest version

{}PERFORMANCE:{}
    • 2x faster than standard rm for large directories
    • Parallel processing using Rayon
    • Optimized for both small and large files
    • Minimal memory footprint

{}WARNING:{}
    This tool permanently deletes files. Use with caution!
    Always double-check the path before running.
{}",
        color::Fg(color::LightGreen),
        env!("CARGO_PKG_VERSION"),
        style::Reset,
        color::Fg(color::LightCyan),
        style::Reset,
        color::Fg(color::LightCyan),
        style::Reset,
        color::Fg(color::LightCyan),
        style::Reset,
        color::Fg(color::LightCyan),
        style::Reset,
        color::Fg(color::LightCyan),
        style::Reset,
        color::Fg(color::LightRed),
        style::Reset,
        style::Reset
    );
}

pub fn print_about() {
    println!(
        "{}rmx - Blazing Fast File Deletion Tool{}

A high-performance alternative to 'rm' written in Rust.
Uses parallel processing for maximum speed.

{}Features:{}
  • Full rm compatibility (-r, -f, -i, -v, -d)
  • Parallel file deletion with Rayon
  • 2x faster than rm for medium-large files
  • Interactive mode for safety
  • Verbose mode for visibility
  • Multiple file/directory support
  • Cross-platform (Linux & macOS)

{}Performance Benchmarks:{}
  • 1K files × 10MB:  ~2x faster than rm
  • 50K small files:  ~2x faster than rm
  • Large directories: ~2x faster than rm
  • Memory usage:     50% less than rm
{}",
        color::Fg(color::LightGreen),
        style::Reset,
        color::Fg(color::LightCyan),
        style::Reset,
        color::Fg(color::LightCyan),
        style::Reset,
        style::Reset
    );
}

pub fn print_dev() {
    println!(
        "{}Developer Information{}

{}Author:{}       Otabek Ismoilov
{}Email:{}        ismoilovdev@gmail.com
{}GitHub:{}       @ismoilovdevml
{}Repository:{}   https://github.com/ismoilovdevml/rmx
{}License:{}      MIT
{}Version:{}      v{}
{}",
        color::Fg(color::LightGreen),
        style::Reset,
        color::Fg(color::LightCyan),
        style::Reset,
        color::Fg(color::LightCyan),
        style::Reset,
        color::Fg(color::LightCyan),
        style::Reset,
        color::Fg(color::LightCyan),
        style::Reset,
        color::Fg(color::LightCyan),
        style::Reset,
        color::Fg(color::LightCyan),
        style::Reset,
        env!("CARGO_PKG_VERSION"),
        style::Reset
    );
}
