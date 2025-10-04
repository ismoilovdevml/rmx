use rayon::prelude::*;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct DeleteStats {
    pub files_deleted: usize,
    pub dirs_deleted: usize,
    pub total_size: u64,
}

impl Default for DeleteStats {
    fn default() -> Self {
        Self::new()
    }
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

/// Atomic counters for high-performance tracking
struct AtomicStats {
    files: AtomicUsize,
    dirs: AtomicUsize,
    size: AtomicU64,
}

impl AtomicStats {
    fn new() -> Self {
        AtomicStats {
            files: AtomicUsize::new(0),
            dirs: AtomicUsize::new(0),
            size: AtomicU64::new(0),
        }
    }

    fn to_delete_stats(&self) -> DeleteStats {
        DeleteStats {
            files_deleted: self.files.load(Ordering::Relaxed),
            dirs_deleted: self.dirs.load(Ordering::Relaxed),
            total_size: self.size.load(Ordering::Relaxed),
        }
    }
}

/// Remove a single file (optimized)
pub fn remove_file(path: &Path, verbose: bool, force: bool) -> Result<DeleteStats, String> {
    let mut stats = DeleteStats::new();

    // Get file size for statistics (always track size)
    let size = match fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(e) => {
            if force {
                return Ok(stats);
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

/// Ultra-fast directory removal
pub fn remove_directory_recursive(
    path: &Path,
    verbose: bool,
    force: bool,
) -> Result<DeleteStats, String> {
    if !path.is_dir() {
        return Ok(DeleteStats::new());
    }

    let stats = Arc::new(AtomicStats::new());

    // Use optimized recursive function
    remove_dir_recursive_fast(path, verbose, force, &stats)?;

    Ok(stats.to_delete_stats())
}

/// Fast recursive directory removal with adaptive parallelism
fn remove_dir_recursive_fast(
    path: &Path,
    verbose: bool,
    force: bool,
    stats: &Arc<AtomicStats>,
) -> Result<(), String> {
    // Read directory entries
    let entries: Vec<_> = match fs::read_dir(path) {
        Ok(entries) => entries.filter_map(|e| e.ok()).collect(),
        Err(e) => {
            if force {
                return Ok(());
            }
            return Err(format!(
                "Failed to read directory '{}': {}",
                path.display(),
                e
            ));
        }
    };

    // Adaptive parallelism threshold based on benchmarking
    // For macOS APFS: parallel is beneficial at 1000+ files
    const PARALLEL_THRESHOLD: usize = 1000;

    if entries.len() >= PARALLEL_THRESHOLD {
        // Use parallel processing for large directories
        let results: Vec<Result<(), String>> = entries
            .par_iter()
            .map(|entry| process_entry_fast(entry, verbose, force, stats))
            .collect();

        // Check for errors
        for result in results {
            if let Err(e) = result {
                if !force {
                    return Err(e);
                }
            }
        }
    } else {
        // Sequential processing for small/medium directories
        for entry in &entries {
            if let Err(e) = process_entry_fast(entry, verbose, force, stats) {
                if !force {
                    return Err(e);
                }
            }
        }
    }

    Ok(())
}

/// Process a single directory entry (highly optimized)
#[inline(always)]
fn process_entry_fast(
    entry: &fs::DirEntry,
    verbose: bool,
    force: bool,
    stats: &Arc<AtomicStats>,
) -> Result<(), String> {
    let path = entry.path();

    // Use DirEntry::metadata() which is cached on most systems
    let metadata = match entry.metadata() {
        Ok(m) => m,
        Err(e) => {
            if force {
                return Ok(());
            }
            return Err(format!("Cannot access '{}': {}", path.display(), e));
        }
    };

    if metadata.is_dir() {
        // Recursively delete subdirectory
        remove_dir_recursive_fast(&path, verbose, force, stats)?;

        // Remove the directory itself
        match fs::remove_dir(&path) {
            Ok(_) => {
                stats.dirs.fetch_add(1, Ordering::Relaxed);
                if verbose {
                    println!("removed directory '{}'", path.display());
                }
                Ok(())
            }
            Err(e) => {
                if force {
                    Ok(())
                } else {
                    Err(format!(
                        "Cannot remove directory '{}': {}",
                        path.display(),
                        e
                    ))
                }
            }
        }
    } else {
        // File deletion - always track size for statistics
        let size = metadata.len();

        match fs::remove_file(&path) {
            Ok(_) => {
                stats.files.fetch_add(1, Ordering::Relaxed);
                stats.size.fetch_add(size, Ordering::Relaxed);
                if verbose {
                    println!("removed '{}'", path.display());
                }
                Ok(())
            }
            Err(e) => {
                if force {
                    Ok(())
                } else {
                    Err(format!("Cannot remove '{}': {}", path.display(), e))
                }
            }
        }
    }
}

/// Remove empty directory
pub fn remove_empty_directory(
    path: &Path,
    verbose: bool,
    force: bool,
) -> Result<DeleteStats, String> {
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
                Err(format!(
                    "Cannot remove directory '{}': {}",
                    path.display(),
                    e
                ))
            }
        }
    }
}

/// Interactive prompt
pub fn prompt_user(path: &Path, is_dir: bool) -> bool {
    use std::io::{self, Write};

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

pub fn remove_file_interactive(
    path: &Path,
    verbose: bool,
    force: bool,
) -> Result<DeleteStats, String> {
    if prompt_user(path, false) {
        remove_file(path, verbose, force)
    } else {
        Ok(DeleteStats::new())
    }
}

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
                    Err(format!(
                        "Cannot remove directory '{}': {}",
                        path.display(),
                        e
                    ))
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
