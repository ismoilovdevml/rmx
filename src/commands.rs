use std::path::Path;
use std::time::Instant;

use termion::{color, style};

use crate::util::bytes_to_readable;
use rmx_lib::remove_dir_contents;

pub fn delete_directory(path_str: &str) {
    let start_time = Instant::now();
    let path = Path::new(path_str);

    if !path.exists() {
        eprintln!(
            "{}Error: Path '{}' does not exist{}",
            color::Fg(color::LightRed),
            path_str,
            style::Reset
        );
        std::process::exit(1);
    }

    if !path.is_dir() {
        eprintln!(
            "{}Error: '{}' is not a directory{}",
            color::Fg(color::LightRed),
            path_str,
            style::Reset
        );
        std::process::exit(1);
    }

    let (all_files, total_size) = match remove_dir_contents(path) {
        Ok(result) => result,
        Err(e) => {
            eprintln!(
                "{}Error deleting files: {}{}",
                color::Fg(color::LightRed),
                e,
                style::Reset
            );
            std::process::exit(1);
        }
    };

    let elapsed_time = start_time.elapsed();

    if all_files > 0 {
        println!(
            "{}✓ Deleted files: {}{}",
            color::Fg(color::LightGreen),
            all_files,
            style::Reset
        );
        println!(
            "{}✓ Total size: {}{}",
            color::Fg(color::LightGreen),
            bytes_to_readable(total_size),
            style::Reset
        );
        println!(
            "{}✓ Time taken: {:.2?}{}",
            color::Fg(color::LightGreen),
            elapsed_time,
            style::Reset
        );
    } else {
        println!(
            "{}No files to delete in '{}'{}",
            color::Fg(color::Yellow),
            path_str,
            style::Reset
        );
    }
}

pub fn print_version() {
    println!("{}rmx v0.4.0{}", color::Fg(color::LightGreen), style::Reset);
}

pub fn print_help() {
    println!(
        "{}rmx v0.4.0{} - Fast alternative to rm command

{}USAGE:{}
    rmx <PATH>              Delete all files in directory
    rmx --version           Show version
    rmx --help              Show this help message
    rmx about               Show program information
    rmx dev                 Show developer information

{}EXAMPLES:{}
    rmx /tmp/test           Delete all files in /tmp/test
    rmx ./build             Delete all files in ./build directory
    rmx ~/Downloads/temp    Delete all files in ~/Downloads/temp

{}OPTIONS:{}
    -h, --help              Print help information
    -v, --version           Print version information

{}WARNING:{}
    This tool permanently deletes files. Use with caution!
    Always double-check the path before running.
{}",
        color::Fg(color::LightGreen),
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
        "{}rmx - Fast File Deletion Tool{}

A blazing fast alternative to 'rm' written in Rust.
Uses parallel processing for maximum performance.

{}Features:{}
  • Parallel file deletion with Rayon
  • 3.5x faster than rm for large files
  • Detailed statistics (count, size, time)
  • Cross-platform (Linux & macOS)
  • Optimized binary (410KB)

{}Performance:{}
  • 10K files × 100KB: Same speed as rm
  • 1K files × 10MB: 3.5x faster than rm
  • 50K files × 10KB: 2x less memory usage
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
{}Email:{}        contact@ismoilovdev.com
{}GitHub:{}       @ismoilovdevml
{}Website:{}      https://ismoilovdev.com
{}Repository:{}   https://github.com/ismoilovdevml/rmx
{}License:{}      MIT
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
        style::Reset
    );
}
