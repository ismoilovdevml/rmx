use std::fs;
use std::path::Path;
use rayon::prelude::*;

pub fn remove_dir_contents(path: &Path) -> Result<(usize, u64), String> {
    let mut count = 0;
    let mut size = 0;

    if path.is_dir() {
        let entries: Vec<_> = fs::read_dir(path).map_err(|e| format!("Failed to read directory: {}", e))?.collect::<Result<_, _>>().map_err(|e| format!("Failed to read entry: {}", e))?;

        let (sub_count, sub_size) = entries.par_iter().map(|entry| {
            let file_name = entry.file_name().into_string().map_err(|_| "Invalid file name")?;
            if file_name.starts_with('.') || file_name == ".." {
                return Ok::<(usize, u64), String>((0, 0));
            }
            let full_path = path.join(&file_name);
            let entry_type = entry.file_type().map_err(|e| format!("Failed to get file type: {}", e))?;
            if entry_type.is_dir() {
                let (sub_count, sub_size) = remove_dir_contents(&full_path)?;
                fs::remove_dir(full_path).map_err(|e| format!("Failed to remove directory: {}", e))?;
                Ok((sub_count, sub_size))
            } else {
                let file_size = entry.metadata().map_err(|e| format!("Failed to get file metadata: {}", e))?.len();
                fs::remove_file(full_path).map_err(|e| format!("Failed to remove file: {}", e))?;
                Ok((1, file_size))
            }
        }).reduce(|| Ok::<(usize, u64), String>((0, 0)), |a, b| {
            let (a_count, a_size) = a?;
            let (b_count, b_size) = b?;
            Ok((a_count + b_count, a_size + b_size))
        })?;
        count += sub_count;
        size += sub_size;
    }

    Ok((count, size))
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_remove_large_file() {
        let dir = tempdir().unwrap();
        let dir_path = dir.path();
        let file_path = dir_path.join("large_testfile");
        let mut file = File::create(file_path).unwrap();

        // 2 Gigabytes = 2 * 1024 Megabytes
        for _ in 0..(2 * 1024) {
            let large_string = "a".repeat(1024 * 1024);  // 1 Megabyte
            writeln!(file, "{}", large_string).unwrap();
        }

        assert!(dir_path.join("large_testfile").exists());

        let result = remove_dir_contents(dir_path);
        assert!(result.is_ok());

        assert!(std::fs::read_dir(dir_path).unwrap().next().is_none());
    }

    #[test]
    fn test_file_deletion() {
        let dir = tempdir().expect("Failed to create temporary directory");
        let dir_path = dir.path();
        for i in 0..5000 {
            let file_path = dir_path.join(format!("file{}", i));
            File::create(file_path).expect("Failed to create file");
        }
        let (files_removed, _) = remove_dir_contents(&dir_path).unwrap();
        assert_eq!(files_removed, 5000);
        dir.close().expect("Failed to close temporary directory");
    }
}
