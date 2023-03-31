use std::env;
use std::path::Path;
use std::time::Instant;

use termion::{color, style};

use rmx::remove_dir_contents;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "{}The path to the folder is not correct{}",
            color::Fg(color::Red),
            style::Reset
        );
        std::process::exit(-1);
    }

    let command = &args[1];
    match command.as_ref() {
        "rmx" => {
            let start_time = Instant::now();
            let path = Path::new(&args[2]);
            if !path.exists() || !path.is_dir() {
                eprintln!(
                    "{}The path to the folder is not correct{}",
                    color::Fg(color::Red),
                    style::Reset
                );
                std::process::exit(-1);
            }
            let all_files = match remove_dir_contents(&path) {
                Ok(files) => files,
                Err(e) => {
                    eprintln!(
                        "{}Error deleting files: {}{}",
                        color::Fg(color::Red),
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
                    color::Fg(color::Green),
                    all_files,
                    style::Reset
                );
                println!(
                    "{}Time taken to delete: {:?}{}",
                    color::Fg(color::Cyan),
                    elapsed_time,
                    style::Reset
                );
            } else {
                eprintln!(
                    "{}Failed to delete any files{}",
                    color::Fg(color::Red),
                    style::Reset
                );
                std::process::exit(-1);
            }
        }
        "about" => println!(
            "{}A program written in the Rust programming language for deleting large and very large files \n
List of commands\n
- rmx   -- is the main command used to delete files \n
- about -- command that provides information about the program \n
- dev   -- Command that provides information about the Developer and the Program source code{}",
            color::Fg(color::Green),
            style::Reset
        ),
        "dev" => println!(
            "{}Programmer: Otabek Ismoilov \n
Source Code: https://github.com/ismoilovdevml/rmx{}",
            color::Fg(color::Green),
            style::Reset
        ),
        "version" => println!(
            "{}rmx v0.2.0{}",
            color::Fg(color::Green),
            style::Reset
        ),
        _ => println!(
            "{}Unknown command: {}{}",
            color::Fg(color::Red),
            command,
            style::Reset
        ),
    }
}
