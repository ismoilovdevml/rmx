use std::fs;
use std::path::Path;
use rayon::prelude::*;

pub fn remove_dir_contents(path: &Path) -> Result<usize, String> {
    let mut count = 0;

    if path.is_dir() {
        let entries: Vec<_> = fs::read_dir(path).map_err(|e| format!("Failed to read directory: {}", e))?.collect::<Result<_, _>>().map_err(|e| format!("Failed to read entry: {}", e))?;

        count += entries.par_iter().map(|entry| {
            let file_name = entry.file_name().into_string().map_err(|_| "Invalid file name")?;
            if file_name.starts_with('.') || file_name == ".." {
                return Ok::<usize, String>(0);
            }
            let full_path = path.join(&file_name);
            if entry.file_type().map_err(|e| format!("Failed to get file type: {}", e))?.is_dir() {
                let sub_count = remove_dir_contents(&full_path)?;
                fs::remove_dir(full_path).map_err(|e| format!("Failed to remove directory: {}", e))?;
                Ok(sub_count)
            } else {
                fs::remove_file(full_path).map_err(|e| format!("Failed to remove file: {}", e))?;
                Ok(1)
            }
        }).reduce(|| Ok::<usize, String>(0), |a, b| Ok(a? + b?))?;
    }

    Ok(count)
}
