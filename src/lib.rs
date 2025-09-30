use rayon::prelude::*;
use std::fs;
use std::path::Path;

pub fn remove_dir_contents(path: &Path) -> Result<(usize, u64), String> {
    if !path.is_dir() {
        return Ok((0, 0));
    }

    let entries: Vec<_> = fs::read_dir(path)
        .map_err(|e| format!("Failed to read directory: {}", e))?
        .filter_map(|e| e.ok())
        .collect();

    let (count, size) = entries
        .par_iter()
        .filter_map(|entry| {
            let file_name = entry.file_name();
            let file_name_str = file_name.to_string_lossy();

            if file_name_str.starts_with('.') {
                return None;
            }

            let path = entry.path();
            let metadata = entry.metadata().ok()?;

            if metadata.is_dir() {
                let (sub_count, sub_size) = remove_dir_contents(&path).ok()?;
                fs::remove_dir(&path).ok()?;
                Some((sub_count, sub_size))
            } else {
                let file_size = metadata.len();
                fs::remove_file(&path).ok()?;
                Some((1, file_size))
            }
        })
        .reduce(|| (0, 0), |a, b| (a.0 + b.0, a.1 + b.1));

    Ok((count, size))
}
