use std::env;
use std::fs;
use std::path::Path;
use std::time::Instant;

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
                match fs::remove_dir(full_path) {
                    Ok(_) => count += 1,
                    Err(_) => println!("Jildni o'chirishda xatolik"),
                }
            } else {
                match fs::remove_file(full_path) {
                    Ok(_) => count += 1,
                    Err(_) => println!("Fayllarni o'chirishda xatolik"),
                }
            }
        }
    }

    count
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Jildga path to'gri berilmagan");
        std::process::exit(-1);
    }

    let start_time = Instant::now();
    let all_files = remove_dir_contents(&args[1]);
    let elapsed_time = start_time.elapsed();
    println!("O'chirilgan fayllar: {}", all_files);
    println!("O'chirish uchun ketgan vaqt: {:?}", elapsed_time);
}
