use std::env;
use std::fs;
use std::path::Path;
use std::time::Instant;

use termion::{color, style};

fn concat(a: &str, b: &str) -> String {
    let mut con = String::from(a);
    con.push_str(b);
    con
}

fn remove_dir_contents(path: &str) -> i32 {
    let path = Path::new(path);
    let mut count = 0;

    if path.is_dir() {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let file_name = entry.file_name().into_string().unwrap();
            if file_name.starts_with(".") || file_name == ".." {
                continue;
            }
            let full_path = concat(path.to_str().unwrap(), "/");
            let full_path = concat(&full_path, &file_name);
            if entry.file_type().unwrap().is_dir() {
                count += remove_dir_contents(&full_path);
                match fs::remove_dir(full_path.clone()) {
                    Ok(_) => count += 1,
                    Err(_) => {
                        println!("{}Error deleting directory: {}{}",
                                 color::Fg(color::Red),
                                 full_path,
                                 style::Reset);
                    }
                }
            } else {
                match fs::remove_file(full_path.clone()) {
                    Ok(_) => count += 1,
                    Err(_) => {
                        println!("{}Error deleting file: {}{}",
                                 color::Fg(color::Red),
                                 full_path,
                                 style::Reset);
                    }
                }
            }
        }
    }

    count
}


fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("{}The path to the folder is not correct{}", color::Fg(color::Red), style::Reset);
        std::process::exit(-1);
    }

    let command = &args[1];
    match command.as_ref() {
        "rmx" => {
            let start_time = Instant::now();
            let all_files = remove_dir_contents(&args[2]);
            let elapsed_time = start_time.elapsed();
            println!("{}Deleted files: {}{}",
                     color::Fg(color::Green),
                     all_files,
                     style::Reset);
            println!("{}Time taken to delete: {:?}{}",
                     color::Fg(color::Cyan),
                     elapsed_time,
                     style::Reset);
        }
        "about" => println!("{}A program written in the Rust programming language for deleting large and very large files \n
List of commands\n
- rmx  -- is the main command used to delete files \n
- about -- command that provides information about the program \n
- dev  -- Command that provides information about the Developer and the Program source code{}",
                            color::Fg(color::Yellow),
                            style::Reset),
        "dev" => println!("{}Programmer: Otabek Ismoilov \n
Source Code: https://github.com/ismoilovdevml/rmx{}",
                          color::Fg(color::Magenta),
                          style::Reset),
        _ => println!("{}Unknown command: {}{}",
                       color::Fg(color::Red),
                       command,
                       style::Reset),
    }
}
