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
