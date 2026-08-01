use rmx_lib::{remove_directory_recursive_with, remove_file, Options};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Unique scratch directory per test, cleaned up on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "rmx-test-{}-{}-{}",
            tag,
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_file(path: &Path, bytes: usize) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, vec![b'x'; bytes]).unwrap();
}

#[test]
fn removes_flat_directory_and_counts_bytes() {
    let tmp = TempDir::new("flat");
    for i in 0..50 {
        write_file(&tmp.path().join(format!("f{i}.txt")), 100);
    }

    let stats = remove_directory_recursive_with(tmp.path(), &Options::default()).unwrap();

    assert_eq!(stats.files_deleted, 50);
    assert_eq!(stats.total_size, 5000);
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
}

/// Exercises the parallel path: must be above FILE_PARALLEL_THRESHOLD (128).
#[test]
fn removes_large_directory_via_parallel_path() {
    let tmp = TempDir::new("parallel");
    for i in 0..1500 {
        write_file(&tmp.path().join(format!("f{i}.txt")), 10);
    }

    let stats = remove_directory_recursive_with(tmp.path(), &Options::default()).unwrap();

    assert_eq!(stats.files_deleted, 1500);
    assert_eq!(stats.total_size, 15_000);
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
}

#[test]
fn removes_nested_tree_and_counts_dirs() {
    let tmp = TempDir::new("nested");
    write_file(&tmp.path().join("a/b/c/deep.txt"), 10);
    write_file(&tmp.path().join("a/b/other.txt"), 10);
    write_file(&tmp.path().join("top.txt"), 10);

    let stats = remove_directory_recursive_with(tmp.path(), &Options::default()).unwrap();

    assert_eq!(stats.files_deleted, 3);
    assert_eq!(stats.dirs_deleted, 3); // a, a/b, a/b/c
    assert_eq!(stats.total_size, 30);
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
}

#[test]
fn track_size_off_still_deletes_but_reports_no_bytes() {
    let tmp = TempDir::new("nosize");
    for i in 0..200 {
        write_file(&tmp.path().join(format!("f{i}.txt")), 64);
    }

    let opts = Options {
        track_size: false,
        ..Options::default()
    };
    let stats = remove_directory_recursive_with(tmp.path(), &opts).unwrap();

    assert_eq!(stats.files_deleted, 200);
    assert_eq!(stats.total_size, 0);
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
}

#[test]
fn missing_path_errors_without_force_and_is_ignored_with_force() {
    let tmp = TempDir::new("missing");
    let ghost = tmp.path().join("does-not-exist.txt");

    assert!(remove_file(&ghost, false, false).is_err());

    let stats = remove_file(&ghost, false, true).unwrap();
    assert_eq!(stats.files_deleted, 0);
}

// -- symlink handling -------------------------------------------------------
// A symlink must be unlinked, never followed. Getting this wrong destroys data
// outside the tree the user asked to delete.

#[cfg(unix)]
#[test]
fn symlinked_directory_is_unlinked_not_followed() {
    let tmp = TempDir::new("symdir");

    // Payload lives outside the tree being deleted.
    let outside = tmp.path().join("outside");
    write_file(&outside.join("precious.txt"), 10);

    let victim = tmp.path().join("victim");
    fs::create_dir_all(&victim).unwrap();
    std::os::unix::fs::symlink(&outside, victim.join("link")).unwrap();

    let stats = remove_directory_recursive_with(&victim, &Options::default()).unwrap();

    // The link itself counts as one removed entry...
    assert_eq!(stats.files_deleted, 1);
    assert_eq!(stats.dirs_deleted, 0);
    // ...and the target survives untouched.
    assert!(outside.join("precious.txt").exists());
}

#[cfg(unix)]
#[test]
fn broken_symlink_is_removable() {
    let tmp = TempDir::new("broken");
    let link = tmp.path().join("dangling");
    std::os::unix::fs::symlink(tmp.path().join("nope"), &link).unwrap();

    // metadata() would fail here (it resolves the target); symlink_metadata
    // does not, so the link is removable without -f.
    let stats = remove_file(&link, false, false).unwrap();

    assert_eq!(stats.files_deleted, 1);
    assert!(fs::symlink_metadata(&link).is_err());
}

#[cfg(unix)]
#[test]
fn symlink_size_is_the_link_not_the_target() {
    let tmp = TempDir::new("symsize");
    let target = tmp.path().join("big.bin");
    write_file(&target, 100_000);

    let dir = tmp.path().join("d");
    fs::create_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink(&target, dir.join("link")).unwrap();

    let stats = remove_directory_recursive_with(&dir, &Options::default()).unwrap();

    assert_eq!(stats.files_deleted, 1);
    assert!(
        stats.total_size < 1000,
        "counted the target's size ({}) instead of the link's",
        stats.total_size
    );
    assert!(target.exists());
}
