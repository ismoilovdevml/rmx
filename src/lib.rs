use rayon::prelude::*;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

#[derive(Debug)]
pub struct DeleteStats {
    pub files_deleted: usize,
    pub dirs_deleted: usize,
    pub total_size: u64,
}

impl DeleteStats {
    pub fn new() -> Self {
        DeleteStats {
            files_deleted: 0,
            dirs_deleted: 0,
            total_size: 0,
        }
    }

    pub fn merge(&mut self, other: DeleteStats) {
        self.files_deleted += other.files_deleted;
        self.dirs_deleted += other.dirs_deleted;
        self.total_size += other.total_size;
    }
}

/// Remove a single file
pub fn remove_file(path: &Path, verbose: bool, force: bool) -> Result<DeleteStats, String> {
    let mut stats = DeleteStats::new();

    // Get file size before deletion
    let size = match fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(e) => {
            if force {
                return Ok(stats); // Silent fail if force is enabled
            }
            return Err(format!("Cannot access '{}': {}", path.display(), e));
        }
    };

    // Delete the file
    match fs::remove_file(path) {
        Ok(_) => {
            stats.files_deleted = 1;
            stats.total_size = size;
            if verbose {
                println!("removed '{}'", path.display());
            }
            Ok(stats)
        }
        Err(e) => {
            if force {
                Ok(stats)
            } else {
                Err(format!("Cannot remove '{}': {}", path.display(), e))
            }
        }
    }
}

/// Remove a directory (recursive)
pub fn remove_directory_recursive(
    path: &Path,
    verbose: bool,
    force: bool,
) -> Result<DeleteStats, String> {
    let mut total_stats = DeleteStats::new();

    if !path.is_dir() {
        return Ok(total_stats);
    }

    // Read directory entries
    let entries: Vec<_> = match fs::read_dir(path) {
        Ok(entries) => entries.filter_map(|e| e.ok()).collect(),
        Err(e) => {
            if force {
                return Ok(total_stats);
            }
            return Err(format!("Failed to read directory '{}': {}", path.display(), e));
        }
    };

    // Process entries in parallel using rayon
    let results: Vec<Result<DeleteStats, String>> = entries
        .par_iter()
        .map(|entry| {
            let path = entry.path();
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(e) => {
                    if force {
                        return Ok(DeleteStats::new());
                    }
                    return Err(format!("Cannot access '{}': {}", path.display(), e));
                }
            };

            if metadata.is_dir() {
                // Recursively delete subdirectory
                let mut stats = remove_directory_recursive(&path, verbose, force)?;
                // Remove the directory itself
                match fs::remove_dir(&path) {
                    Ok(_) => {
                        stats.dirs_deleted += 1;
                        if verbose {
                            println!("removed directory '{}'", path.display());
                        }
                        Ok(stats)
                    }
                    Err(e) => {
                        if force {
                            Ok(stats)
                        } else {
                            Err(format!("Cannot remove directory '{}': {}", path.display(), e))
                        }
                    }
                }
            } else {
                // Delete file
                remove_file(&path, verbose, force)
            }
        })
        .collect();

    // Merge all stats
    for result in results {
        match result {
            Ok(stats) => total_stats.merge(stats),
            Err(e) => return Err(e),
        }
    }

    Ok(total_stats)
}

/// Remove empty directory
pub fn remove_empty_directory(path: &Path, verbose: bool, force: bool) -> Result<DeleteStats, String> {
    let mut stats = DeleteStats::new();

    match fs::remove_dir(path) {
        Ok(_) => {
            stats.dirs_deleted = 1;
            if verbose {
                println!("removed directory '{}'", path.display());
            }
            Ok(stats)
        }
        Err(e) => {
            if force {
                Ok(stats)
            } else {
                Err(format!("Cannot remove directory '{}': {}", path.display(), e))
            }
        }
    }
}

/// Interactive prompt
pub fn prompt_user(path: &Path, is_dir: bool) -> bool {
    let prompt = if is_dir {
        format!("remove directory '{}'? ", path.display())
    } else {
        format!("remove file '{}'? ", path.display())
    };

    print!("{}", prompt);
    io::stdout().flush().unwrap();

    let mut input = String::new();
    match io::stdin().read_line(&mut input) {
        Ok(_) => {
            let answer = input.trim().to_lowercase();
            answer == "y" || answer == "yes"
        }
        Err(_) => false,
    }
}

/// Remove file with interactive mode
pub fn remove_file_interactive(path: &Path, verbose: bool, force: bool) -> Result<DeleteStats, String> {
    if prompt_user(path, false) {
        remove_file(path, verbose, force)
    } else {
        Ok(DeleteStats::new())
    }
}

/// Remove directory with interactive mode
pub fn remove_directory_interactive(
    path: &Path,
    verbose: bool,
    force: bool,
) -> Result<DeleteStats, String> {
    if prompt_user(path, true) {
        let mut stats = remove_directory_recursive(path, verbose, force)?;
        match fs::remove_dir(path) {
            Ok(_) => {
                stats.dirs_deleted += 1;
                if verbose {
                    println!("removed directory '{}'", path.display());
                }
                Ok(stats)
            }
            Err(e) => {
                if force {
                    Ok(stats)
                } else {
                    Err(format!("Cannot remove directory '{}': {}", path.display(), e))
                }
            }
        }
    } else {
        Ok(DeleteStats::new())
    }
}

/// Legacy function for backwards compatibility
pub fn remove_dir_contents(path: &Path) -> Result<(usize, u64), String> {
    let stats = remove_directory_recursive(path, false, false)?;
    Ok((stats.files_deleted, stats.total_size))
}
