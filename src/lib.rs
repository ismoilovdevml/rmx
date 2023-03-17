use std::fs;
use std::path::{Path};

pub fn remove_dir_contents(path: &Path) -> Result<usize, String> {
    let mut count = 0;

    if path.is_dir() {
        for entry in fs::read_dir(path).map_err(|e| format!("Failed to read directory: {}", e))? {
            let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
            let file_name = entry.file_name().into_string().map_err(|_| "Invalid file name")?;
            if file_name.starts_with('.') || file_name == ".." {
                continue;
            }
            let full_path = path.join(&file_name);
            if entry.file_type().map_err(|e| format!("Failed to get file type: {}", e))?.is_dir() {
                count += remove_dir_contents(&full_path)?;
                fs::remove_dir(full_path).map_err(|e| format!("Failed to remove directory: {}", e))?;
            } else {
                fs::remove_file(full_path).map_err(|e| format!("Failed to remove file: {}", e))?;
                count += 1;
            }
        }
    }

    Ok(count)
}
