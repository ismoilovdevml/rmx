use std::path::Path;
use std::time::Instant;

use termion::{color, style};

use crate::util::bytes_to_readable;
use rmx_lib::remove_dir_contents;

pub fn execute_command(command: &str, path_str: Option<&str>) {
    match command {
        "rmx" => {
            if let Some(path_str) = path_str {
                let start_time = Instant::now();
                let path = Path::new(path_str);
                if !path.exists() || !path.is_dir() {
                    eprintln!(
                        "{}The path to the folder is not correct{}",
                        color::Fg(color::LightRed),
                        style::Reset
                    );
                    std::process::exit(-1);
                }
                let (all_files, total_size) = match remove_dir_contents(&path) {
                    Ok(result) => result,
                    Err(e) => {
                        eprintln!(
                            "{}Error deleting files: {}{}",
                            color::Fg(color::LightRed),
                            e,
                            style::Reset
                        );
                        std::process::exit(-1);
                    }
                };
                let elapsed_time = start_time.elapsed();
                if all_files > 0 {
                    println!(
                        "{}Deleted files: {}{}",
                        color::Fg(color::LightGreen),
                        all_files,
                        style::Reset
                    );
                    println!(
                        "{}Total size of deleted files: {}{}",
                        color::Fg(color::LightGreen),
                        bytes_to_readable(total_size),
                        style::Reset
                    );
                    println!(
                        "{}Time taken to delete: {:?}{}",
                        color::Fg(color::LightGreen),
                        elapsed_time,
                        style::Reset
                    );
                } else {
                    eprintln!(
                        "{}Failed to delete any files{}",
                        color::Fg(color::LightRed),
                        style::Reset
                    );
                    std::process::exit(-1);
                }
            } else {
                eprintln!(
                    "{}The path to the folder is not provided{}",
                    color::Fg(color::LightRed),
                    style::Reset
                );
                std::process::exit(-1);
            }
        },
        "about" => println!(
            "{}A program written in the Rust programming language for deleting large and very large files \n
List of commands\n
- rmx   -- is the main command used to delete files \n
- about -- command that provides information about the program \n
- dev   -- Command that provides information about the Developer and the Program source code{}",
            color::Fg(color::LightGreen),
            style::Reset
        ),
        "dev" => println!(
            "{}Programmer: Otabek Ismoilov \n
Source Code: https://github.com/ismoilovdevml/rmx{}",
            color::Fg(color::LightGreen),
            style::Reset
        ),
        "version" => println!(
            "{}rmx v0.3.0{}",
            color::Fg(color::LightGreen),
            style::Reset
        ),
        _ => println!(
            "{}Unknown command: {}{}",
            color::Fg(color::LightRed),
            command,
            style::Reset
        ),
    }
}