use rmx_lib::{
    remove_all, remove_all_interactive, remove_directory_recursive_with, remove_file, DeleteStats,
    Options, Progress,
};
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

// -- remove_all ---------------------------------------------------------------

#[cfg(unix)]
fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() == 0 }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

#[test]
fn remove_all_removes_nested_tree_including_operand() {
    let tmp = TempDir::new("all-nested");
    let root = tmp.path().join("build");
    write_file(&root.join("a/b/c/deep.txt"), 10);
    write_file(&root.join("a/b/other.txt"), 20);
    write_file(&root.join("top.txt"), 30);
    fs::create_dir_all(root.join("empty/inner")).unwrap();

    let out = remove_all(std::slice::from_ref(&root), &Options::default());

    assert!(out.is_ok(), "{:?}", out.errors);
    assert_eq!(out.stats.files_deleted, 3);
    // build, a, a/b, a/b/c, empty, empty/inner
    assert_eq!(out.stats.dirs_deleted, 6);
    assert_eq!(out.stats.total_size, 60);
    assert!(!exists(&root));
}

fn make_node_modules(root: &Path) {
    for d in 0..120 {
        for f in 0..12 {
            write_file(&root.join(format!("pkg{d}/lib/f{f}.js")), 7);
        }
    }
}

#[test]
fn remove_all_handles_many_directories_through_the_pool() {
    // node_modules shape: many small directories, many batches.
    let tmp = TempDir::new("all-pool");
    let root = tmp.path().join("nm");
    for jobs in [0, 1, 3] {
        make_node_modules(&root);
        let opts = Options {
            jobs,
            ..Options::default()
        };
        let out = remove_all(std::slice::from_ref(&root), &opts);
        assert!(out.is_ok(), "jobs={jobs}: {:?}", out.errors);
        assert_eq!(out.stats.files_deleted, 1440, "jobs={jobs}");
        assert_eq!(out.stats.dirs_deleted, 241, "jobs={jobs}");
        assert_eq!(out.stats.total_size, 1440 * 7, "jobs={jobs}");
        assert!(!exists(&root));
    }
}

#[test]
fn remove_all_top_level_file_operands() {
    let tmp = TempDir::new("all-files");
    let mut paths = Vec::new();
    for i in 0..600 {
        let p = tmp.path().join(format!("f{i}.log"));
        write_file(&p, 3);
        paths.push(p);
    }
    let out = remove_all(&paths, &Options::default());
    assert!(out.is_ok(), "{:?}", out.errors);
    assert_eq!(out.stats.files_deleted, 600);
    assert_eq!(out.stats.total_size, 1800);
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn remove_all_missing_operand_errors_unless_force() {
    let tmp = TempDir::new("all-missing");
    let ghost = tmp.path().join("ghost");
    let real = tmp.path().join("real.txt");
    write_file(&real, 1);

    let out = remove_all(&[ghost.clone(), real.clone()], &Options::default());
    assert_eq!(
        out.errors,
        vec![format!(
            "cannot remove '{}': No such file or directory",
            ghost.display()
        )]
    );
    // The error did not stop the next operand.
    assert!(!exists(&real));

    let forced = Options {
        force: true,
        ..Options::default()
    };
    let out = remove_all(&[ghost], &forced);
    assert!(out.is_ok(), "{:?}", out.errors);
}

#[cfg(unix)]
#[test]
fn non_recursive_directory_is_an_error() {
    let tmp = TempDir::new("all-isdir");
    let d = tmp.path().join("d");
    write_file(&d.join("f"), 1);
    let opts = Options {
        recursive: false,
        ..Options::default()
    };
    let out = remove_all(std::slice::from_ref(&d), &opts);
    assert_eq!(
        out.errors,
        vec![format!("cannot remove '{}': Is a directory", d.display())]
    );
    assert!(d.join("f").exists());
}

#[cfg(unix)]
#[test]
fn dir_flag_removes_empty_and_refuses_non_empty() {
    let tmp = TempDir::new("all-d");
    let empty = tmp.path().join("empty");
    let full = tmp.path().join("full");
    fs::create_dir_all(&empty).unwrap();
    write_file(&full.join("f"), 1);
    let opts = Options {
        recursive: false,
        dir: true,
        ..Options::default()
    };
    let out = remove_all(&[empty.clone(), full.clone()], &opts);
    assert!(!exists(&empty));
    assert_eq!(out.stats.dirs_deleted, 1);
    assert_eq!(
        out.errors,
        vec![format!(
            "cannot remove '{}': Directory not empty",
            full.display()
        )]
    );
    assert!(full.join("f").exists());
}

#[cfg(unix)]
#[test]
fn force_still_reports_permission_errors_and_removes_siblings() {
    if is_root() {
        return; // root ignores directory permissions
    }
    let tmp = TempDir::new("all-locked");
    let root = tmp.path().join("tree");
    let locked = root.join("locked");
    write_file(&locked.join("f"), 1);
    for i in 0..300 {
        write_file(&root.join(format!("sib/s{i}")), 1);
    }
    write_file(&root.join("x.txt"), 1);
    set_mode(&locked, 0o555);

    let opts = Options {
        force: true,
        ..Options::default()
    };
    let out = remove_all(std::slice::from_ref(&root), &opts);
    set_mode(&locked, 0o755);

    // Exactly one error: the file. No spurious "Directory not empty" for
    // `locked` or `tree`, which simply stay.
    assert_eq!(
        out.errors,
        vec![format!(
            "cannot remove '{}': Permission denied",
            locked.join("f").display()
        )]
    );
    assert!(locked.join("f").exists());
    assert!(!exists(&root.join("sib")));
    assert!(!exists(&root.join("x.txt")));
    assert_eq!(out.stats.files_deleted, 301);
}

#[cfg(unix)]
#[test]
fn unreadable_directories_are_removed_when_empty() {
    if is_root() {
        return;
    }
    let tmp = TempDir::new("all-0000");
    let root = tmp.path().join("tree");
    let inner_empty = root.join("sealed");
    fs::create_dir_all(&inner_empty).unwrap();
    write_file(&root.join("f"), 1);
    set_mode(&inner_empty, 0o000);
    let top_empty = tmp.path().join("sealed-top");
    fs::create_dir_all(&top_empty).unwrap();
    set_mode(&top_empty, 0o000);

    let out = remove_all(&[root.clone(), top_empty.clone()], &Options::default());
    assert!(out.is_ok(), "{:?}", out.errors);
    assert!(!exists(&root));
    assert!(!exists(&top_empty));
}

#[cfg(unix)]
#[test]
fn unreadable_non_empty_directory_reports_open_error() {
    if is_root() {
        return;
    }
    let tmp = TempDir::new("all-0000-full");
    let root = tmp.path().join("tree");
    let sealed = root.join("sealed");
    write_file(&sealed.join("f"), 1);
    write_file(&root.join("g"), 1);
    set_mode(&sealed, 0o000);

    let out = remove_all(std::slice::from_ref(&root), &Options::default());
    set_mode(&sealed, 0o755);
    assert_eq!(
        out.errors,
        vec![format!(
            "cannot remove '{}': Permission denied",
            sealed.display()
        )]
    );
    assert!(sealed.join("f").exists());
    assert!(!exists(&root.join("g")));
}

#[cfg(unix)]
#[test]
fn symlink_to_directory_operand_is_unlinked_not_followed() {
    let tmp = TempDir::new("all-symop");
    let target = tmp.path().join("target");
    write_file(&target.join("precious.txt"), 5);
    let link = tmp.path().join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let out = remove_all(std::slice::from_ref(&link), &Options::default());
    assert!(out.is_ok(), "{:?}", out.errors);
    assert!(!exists(&link));
    assert!(target.join("precious.txt").exists());
    assert_eq!(out.stats.files_deleted, 1);
}

#[cfg(unix)]
#[test]
fn symlink_inside_tree_is_not_followed() {
    let tmp = TempDir::new("all-symin");
    let outside = tmp.path().join("outside");
    write_file(&outside.join("precious.txt"), 5);
    let root = tmp.path().join("tree");
    write_file(&root.join("a/f"), 1);
    std::os::unix::fs::symlink(&outside, root.join("a/link")).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("link2")).unwrap();

    let out = remove_all(std::slice::from_ref(&root), &Options::default());
    assert!(out.is_ok(), "{:?}", out.errors);
    assert!(!exists(&root));
    assert!(outside.join("precious.txt").exists());
    assert_eq!(out.stats.files_deleted, 3);
}

#[cfg(unix)]
#[test]
fn hard_links_are_counted_once() {
    let tmp = TempDir::new("all-hardlink");
    let root = tmp.path().join("store");
    write_file(&root.join("a/blob"), 1000);
    for i in 0..3 {
        fs::create_dir_all(root.join(format!("pkg{i}"))).unwrap();
        fs::hard_link(root.join("a/blob"), root.join(format!("pkg{i}/blob"))).unwrap();
    }
    write_file(&root.join("plain"), 10);

    let out = remove_all(std::slice::from_ref(&root), &Options::default());
    assert!(out.is_ok(), "{:?}", out.errors);
    assert_eq!(out.stats.files_deleted, 5);
    assert_eq!(out.stats.total_size, 1010);
    assert!(!exists(&root));
}

#[cfg(unix)]
/// Build `depth` nested directories named "d" with fd-relative calls, since
/// the full path is far beyond PATH_MAX. Every 100th level gets a file.
fn make_deep(root: &Path, depth: usize) {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    fs::create_dir_all(root).unwrap();
    let c = CString::new(root.as_os_str().as_bytes()).unwrap();
    let name = CString::new("d").unwrap();
    let file = CString::new("f").unwrap();
    // SAFETY: plain fd-relative syscalls on valid C strings.
    unsafe {
        let mut fd = libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY);
        assert!(fd >= 0);
        for i in 0..depth {
            assert_eq!(libc::mkdirat(fd, name.as_ptr(), 0o755), 0);
            if i % 100 == 0 {
                let f = libc::openat(
                    fd,
                    file.as_ptr(),
                    libc::O_CREAT | libc::O_WRONLY,
                    0o644 as libc::c_uint,
                );
                assert!(f >= 0);
                libc::close(f);
            }
            let next = libc::openat(fd, name.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY);
            assert!(next >= 0);
            libc::close(fd);
            fd = next;
        }
        libc::close(fd);
    }
}

#[cfg(unix)]
#[test]
fn deep_tree_no_stack_overflow_no_emfile() {
    let tmp = TempDir::new("all-deep");
    let root = tmp.path().join("deep");
    make_deep(&root, 3000);

    let out = remove_all(std::slice::from_ref(&root), &Options::default());
    assert!(out.is_ok(), "{:?}", out.errors);
    assert_eq!(out.stats.dirs_deleted, 3001);
    assert_eq!(out.stats.files_deleted, 30);
    assert!(!exists(&root));
}

#[cfg(unix)]
#[test]
fn deeper_than_budget_proceeds_instead_of_hanging() {
    // The stack alone exceeds the budget: the walker must not wait for
    // directories that only it can release.
    let tmp = TempDir::new("all-deep-budget");
    let root = tmp.path().join("deep");
    make_deep(&root, 200);

    rmx_lib::__set_open_dir_budget(Some(32));
    let out = remove_all(std::slice::from_ref(&root), &Options::default());
    rmx_lib::__set_open_dir_budget(None);
    assert!(out.is_ok(), "{:?}", out.errors);
    assert!(!exists(&root));
}

fn make_wide(root: &Path) {
    for i in 0..3000 {
        write_file(&root.join(format!("d{i}/f")), 1);
    }
}

#[cfg(unix)]
#[test]
fn wide_tree_stays_within_open_directory_budget() {
    let tmp = TempDir::new("all-wide");
    let root = tmp.path().join("wide");
    make_wide(&root);

    // 64 is far below the 256 that macOS hands out by default.
    rmx_lib::__set_open_dir_budget(Some(64));
    let out = remove_all(std::slice::from_ref(&root), &Options::default());
    let peak = rmx_lib::__last_peak_open_dirs();
    rmx_lib::__set_open_dir_budget(None);

    assert!(out.is_ok(), "{:?}", out.errors);
    assert_eq!(out.stats.files_deleted, 3000);
    assert_eq!(out.stats.dirs_deleted, 3001);
    assert!(!exists(&root));
    assert!(peak <= 64, "peak open directories {peak} exceeded budget");
}

#[test]
fn wide_tree_with_default_budget() {
    let tmp = TempDir::new("all-wide-default");
    let root = tmp.path().join("wide");
    make_wide(&root);
    let out = remove_all(std::slice::from_ref(&root), &Options::default());
    assert!(out.is_ok(), "{:?}", out.errors);
    assert_eq!(out.stats.dirs_deleted, 3001);
    assert!(!exists(&root));
}

#[cfg(unix)]
/// APFS rejects names that are not valid UTF-8; ext4 and friends accept them.
fn accepts_raw_bytes(dir: &Path) -> bool {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let probe = dir.join(OsStr::from_bytes(b"probe-\xff"));
    let ok = fs::write(&probe, b"").is_ok();
    let _ = fs::remove_file(&probe);
    ok
}

#[cfg(unix)]
#[test]
fn non_utf8_names_are_removed() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let tmp = TempDir::new("all-bytes");
    let root = tmp.path().join("tree");
    fs::create_dir_all(&root).unwrap();
    let raw: &[u8] = if accepts_raw_bytes(&root) {
        b"caf\xe9-\xff"
    } else {
        // Still exercise non-ASCII multi-byte names.
        "café-日本".as_bytes()
    };
    let name = OsStr::from_bytes(raw);
    write_file(&root.join(name).join(name), 4);
    write_file(&root.join(name).with_extension("txt"), 4);

    let out = remove_all(std::slice::from_ref(&root), &Options::default());
    assert!(out.is_ok(), "{:?}", out.errors);
    assert_eq!(out.stats.files_deleted, 2);
    assert_eq!(out.stats.dirs_deleted, 2);
    assert!(!exists(&root));
}

#[test]
fn errors_have_no_prefix_or_os_error_suffix() {
    let tmp = TempDir::new("all-suffix");
    let out = remove_all(&[tmp.path().join("nope")], &Options::default());
    assert_eq!(out.errors.len(), 1);
    assert!(!out.errors[0].contains("os error"), "{}", out.errors[0]);
    assert!(!out.errors[0].starts_with("rmx:"), "{}", out.errors[0]);
}

#[test]
fn legacy_recursive_on_a_file_is_an_error() {
    let tmp = TempDir::new("legacy-file");
    let f = tmp.path().join("f");
    write_file(&f, 1);
    assert!(remove_directory_recursive_with(&f, &Options::default()).is_err());
    assert!(f.exists());
}

// -- interactive --------------------------------------------------------------

#[cfg(unix)]
fn interactive_tree(tmp: &TempDir) -> PathBuf {
    let root = tmp.path().join("d");
    write_file(&root.join("f"), 3);
    write_file(&root.join("sub/g"), 4);
    std::os::unix::fs::symlink("nowhere", root.join("l")).unwrap();
    root
}

#[cfg(unix)]
#[test]
fn interactive_all_yes_removes_everything() {
    let tmp = TempDir::new("int-yes");
    let root = interactive_tree(&tmp);
    let mut asked = Vec::new();
    let out = remove_all_interactive(std::slice::from_ref(&root), &Options::default(), &mut |q| {
        asked.push(q.to_string());
        true
    });
    assert!(out.is_ok(), "{:?}", out.errors);
    assert!(!exists(&root));
    assert_eq!(out.stats.files_deleted, 3);
    assert_eq!(out.stats.dirs_deleted, 2);
    let d = root.display();
    assert_eq!(asked[0], format!("descend into directory '{d}'?"));
    assert_eq!(asked[asked.len() - 1], format!("remove directory '{d}'?"));
    for q in [
        format!("remove regular file '{d}/f'?"),
        format!("remove symbolic link '{d}/l'?"),
        format!("descend into directory '{d}/sub'?"),
        format!("remove regular file '{d}/sub/g'?"),
        format!("remove directory '{d}/sub'?"),
    ] {
        assert!(asked.contains(&q), "missing {q:?} in {asked:?}");
    }
    assert_eq!(asked.len(), 7);
}

#[cfg(unix)]
#[test]
fn interactive_declining_a_file_keeps_it_and_its_parents() {
    let tmp = TempDir::new("int-file");
    let root = interactive_tree(&tmp);
    let keep = format!("remove regular file '{}'?", root.join("sub/g").display());
    let mut asked = Vec::new();
    let out = remove_all_interactive(std::slice::from_ref(&root), &Options::default(), &mut |q| {
        asked.push(q.to_string());
        q != keep
    });
    assert!(out.is_ok(), "{:?}", out.errors);
    assert!(root.join("sub/g").exists());
    assert!(!exists(&root.join("f")));
    assert!(!exists(&root.join("l")));
    // Neither ancestor is offered for removal: they cannot be empty.
    assert!(!asked.iter().any(|q| q.starts_with("remove directory")));
}

#[cfg(unix)]
#[test]
fn interactive_declining_descend_keeps_subtree() {
    let tmp = TempDir::new("int-descend");
    let root = interactive_tree(&tmp);
    let skip = format!("descend into directory '{}'?", root.join("sub").display());
    let out = remove_all_interactive(std::slice::from_ref(&root), &Options::default(), &mut |q| {
        q != skip
    });
    assert!(out.is_ok(), "{:?}", out.errors);
    assert!(root.join("sub/g").exists());
    assert!(!exists(&root.join("f")));
}

#[test]
fn interactive_non_recursive_file_asks_once() {
    let tmp = TempDir::new("int-single");
    let f = tmp.path().join("a.txt");
    write_file(&f, 2);
    let opts = Options {
        recursive: false,
        ..Options::default()
    };
    let mut asked = Vec::new();
    let out = remove_all_interactive(std::slice::from_ref(&f), &opts, &mut |q| {
        asked.push(q.to_string());
        true
    });
    assert!(out.is_ok());
    assert_eq!(
        asked,
        vec![format!("remove regular file '{}'?", f.display())]
    );
    assert!(!exists(&f));
}

#[test]
fn interactive_empty_directory_is_not_descended() {
    // GNU asks only `remove directory` for an empty directory.
    let tmp = TempDir::new("int-empty");
    let top = tmp.path().join("e");
    fs::create_dir_all(&top).unwrap();
    let nested = tmp.path().join("d");
    fs::create_dir_all(nested.join("empty")).unwrap();
    write_file(&nested.join("f"), 1);

    let mut asked = Vec::new();
    let out = remove_all_interactive(
        &[top.clone(), nested.clone()],
        &Options::default(),
        &mut |q| {
            asked.push(q.to_string());
            true
        },
    );
    assert!(out.is_ok(), "{:?}", out.errors);
    assert!(!exists(&top));
    assert!(!exists(&nested));
    let (t, d) = (top.display(), nested.display());
    let empty = nested.join("empty");
    assert_eq!(asked[0], format!("remove directory '{t}'?"));
    assert_eq!(asked[1], format!("descend into directory '{d}'?"));
    assert!(asked.contains(&format!("remove directory '{}'?", empty.display())));
    assert!(!asked.contains(&format!("descend into directory '{}'?", empty.display())));
    assert!(!asked.contains(&format!("descend into directory '{t}'?")));
    assert_eq!(asked.len(), 5, "{asked:?}");
}

// -- dry run ------------------------------------------------------------------

#[cfg(unix)]
fn mode_of(m: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    m.permissions().mode()
}

#[cfg(not(unix))]
fn mode_of(m: &fs::Metadata) -> u32 {
    u32::from(m.permissions().readonly())
}

/// Every entry below `root` with its type, size and permissions, sorted.
/// Entries that cannot be read or stat'ed are listed as such.
fn listing(root: &Path) -> Vec<(PathBuf, &'static str, u64, u32)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else {
            out.push((dir, "unreadable", 0, 0));
            continue;
        };
        for e in rd {
            let p = e.unwrap().path();
            let Ok(m) = fs::symlink_metadata(&p) else {
                out.push((p, "unknown", 0, 0));
                continue;
            };
            let kind = if m.is_dir() {
                stack.push(p.clone());
                "dir"
            } else if m.file_type().is_symlink() {
                "link"
            } else {
                "file"
            };
            let len = if m.is_file() { m.len() } else { 0 };
            out.push((p, kind, len, mode_of(&m)));
        }
    }
    out.sort();
    out
}

fn same_stats(a: &DeleteStats, b: &DeleteStats) -> bool {
    (a.files_deleted, a.dirs_deleted, a.total_size)
        == (b.files_deleted, b.dirs_deleted, b.total_size)
}

#[test]
fn dry_run_leaves_a_big_tree_untouched_and_matches_the_real_run() {
    let tmp = TempDir::new("dry-big");
    let root = tmp.path().join("big");
    make_node_modules(&root);
    // One directory past the inode-sort threshold, nested and empty ones.
    for i in 0..10_050 {
        write_file(&root.join(format!("flat/f{i}")), i % 7);
    }
    fs::create_dir_all(root.join("e1/e2/e3")).unwrap();
    write_file(&root.join("x/y/z/w/v.txt"), 11);
    let loose = tmp.path().join("loose.txt");
    write_file(&loose, 5);
    let paths = vec![root.clone(), loose.clone()];

    let before = listing(tmp.path());
    let mut dry_stats: Option<DeleteStats> = None;
    for jobs in [0, 1, 3] {
        let opts = Options {
            dry_run: true,
            jobs,
            ..Options::default()
        };
        let out = remove_all(&paths, &opts);
        assert!(out.is_ok(), "jobs={jobs}: {:?}", out.errors);
        assert_eq!(listing(tmp.path()), before, "dry run changed the tree");
        assert_eq!(out.stats.files_deleted, 1440 + 10_050 + 1 + 1);
        if let Some(prev) = &dry_stats {
            assert!(same_stats(prev, &out.stats), "{prev:?} vs {:?}", out.stats);
        }
        dry_stats = Some(out.stats);
    }

    let real = remove_all(&paths, &Options::default());
    assert!(real.is_ok(), "{:?}", real.errors);
    let dry = dry_stats.unwrap();
    assert!(
        same_stats(&dry, &real.stats),
        "dry {dry:?} real {:?}",
        real.stats
    );
    assert!(!exists(&root));
    assert!(!exists(&loose));
}

#[test]
fn progress_counters_match_returned_stats() {
    let tmp = TempDir::new("progress");
    let root = tmp.path().join("nm");
    make_node_modules(&root);
    write_file(&root.join("big.bin"), 4096);
    for dry_run in [true, false] {
        let progress: &'static Progress = Box::leak(Box::default());
        let opts = Options {
            dry_run,
            progress: Some(progress),
            ..Options::default()
        };
        let out = remove_all(std::slice::from_ref(&root), &opts);
        assert!(out.is_ok(), "{:?}", out.errors);
        let load = |a: &std::sync::atomic::AtomicU64| a.load(Ordering::Relaxed);
        assert_eq!(load(&progress.files), out.stats.files_deleted as u64);
        assert_eq!(load(&progress.dirs), out.stats.dirs_deleted as u64);
        assert_eq!(load(&progress.bytes), out.stats.total_size);
        assert_eq!(out.stats.files_deleted, 1441);
        assert_eq!(out.stats.dirs_deleted, 241);
        assert_eq!(exists(&root), dry_run);
    }
}

#[test]
fn interactive_dry_run_asks_but_removes_nothing() {
    let tmp = TempDir::new("int-dry");
    let root = tmp.path().join("d");
    write_file(&root.join("f"), 3);
    write_file(&root.join("sub").join("g"), 4);
    fs::create_dir_all(root.join("empty")).unwrap();
    let before = listing(tmp.path());

    let progress: &'static Progress = Box::leak(Box::default());
    let opts = Options {
        dry_run: true,
        progress: Some(progress),
        ..Options::default()
    };
    let mut asked = Vec::new();
    let out = remove_all_interactive(std::slice::from_ref(&root), &opts, &mut |q| {
        asked.push(q.to_string());
        true
    });
    assert!(out.is_ok(), "{:?}", out.errors);
    assert_eq!(listing(tmp.path()), before);
    assert_eq!(out.stats.files_deleted, 2);
    assert_eq!(out.stats.dirs_deleted, 3);
    assert_eq!(out.stats.total_size, 7);
    assert_eq!(progress.files.load(Ordering::Relaxed), 2);
    assert_eq!(progress.dirs.load(Ordering::Relaxed), 3);
    assert_eq!(progress.bytes.load(Ordering::Relaxed), 7);
    assert_eq!(
        asked.last().unwrap(),
        &format!("remove directory '{}'?", root.display())
    );
    // descend d, f, descend sub, g, rm sub, rm empty, rm d
    assert_eq!(asked.len(), 7, "{asked:?}");

    // Declining a file keeps its ancestors, as in a real run.
    let keep = format!(
        "remove regular file '{}'?",
        root.join("sub").join("g").display()
    );
    let mut asked = Vec::new();
    let out = remove_all_interactive(std::slice::from_ref(&root), &opts, &mut |q| {
        asked.push(q.to_string());
        q != keep
    });
    assert!(out.is_ok(), "{:?}", out.errors);
    assert_eq!(out.stats.dirs_deleted, 1); // only `empty`
    assert!(!asked.contains(&format!("remove directory '{}'?", root.display())));
    assert_eq!(listing(tmp.path()), before);
}

#[cfg(unix)]
#[test]
fn dry_run_counts_hard_links_once() {
    let tmp = TempDir::new("dry-hardlink");
    let root = tmp.path().join("store");
    write_file(&root.join("a/blob"), 1000);
    for i in 0..3 {
        fs::create_dir_all(root.join(format!("pkg{i}"))).unwrap();
        fs::hard_link(root.join("a/blob"), root.join(format!("pkg{i}/blob"))).unwrap();
    }
    write_file(&root.join("plain"), 10);
    let before = listing(tmp.path());

    let opts = Options {
        dry_run: true,
        ..Options::default()
    };
    let dry = remove_all(std::slice::from_ref(&root), &opts);
    assert!(dry.is_ok(), "{:?}", dry.errors);
    assert_eq!(listing(tmp.path()), before);
    assert_eq!(dry.stats.files_deleted, 5);
    assert_eq!(dry.stats.total_size, 1010);

    let real = remove_all(std::slice::from_ref(&root), &Options::default());
    assert!(real.is_ok(), "{:?}", real.errors);
    assert!(
        same_stats(&dry.stats, &real.stats),
        "{:?} {:?}",
        dry.stats,
        real.stats
    );
}

#[cfg(unix)]
#[test]
fn dry_run_reports_the_same_errors_as_a_real_run() {
    let tmp = TempDir::new("dry-errors");
    let ghost = tmp.path().join("ghost");
    let dir = tmp.path().join("plain-dir");
    write_file(&dir.join("f"), 1);
    let empty = tmp.path().join("empty");
    fs::create_dir_all(&empty).unwrap();
    let before = listing(tmp.path());

    // Missing operand; directory without -r; -d on a non-empty directory.
    for (paths, recursive, dir_flag) in [
        (vec![ghost.clone()], true, false),
        (vec![dir.clone()], false, false),
        (vec![dir.clone(), empty.clone()], false, true),
    ] {
        let opts = Options {
            recursive,
            dir: dir_flag,
            ..Options::default()
        };
        let dry = remove_all(
            &paths,
            &Options {
                dry_run: true,
                ..opts
            },
        );
        assert_eq!(listing(tmp.path()), before);
        assert!(!dry.errors.is_empty());
        let real = remove_all(&paths, &opts);
        assert_eq!(dry.errors, real.errors);
        assert!(
            same_stats(&dry.stats, &real.stats),
            "{:?} {:?}",
            dry.stats,
            real.stats
        );
    }
    assert!(!exists(&empty)); // the last real run removed it
}

#[cfg(unix)]
#[test]
fn dry_run_reports_unreadable_directories_like_a_real_run() {
    if is_root() {
        return;
    }
    let tmp = TempDir::new("dry-0000");
    let root = tmp.path().join("tree");
    let sealed = root.join("sealed");
    write_file(&sealed.join("f"), 1);
    write_file(&root.join("g"), 1);
    set_mode(&sealed, 0o000);

    let dry = remove_all(
        std::slice::from_ref(&root),
        &Options {
            dry_run: true,
            ..Options::default()
        },
    );
    assert!(exists(&root.join("g")));
    let real = remove_all(std::slice::from_ref(&root), &Options::default());
    set_mode(&sealed, 0o755);
    let expected = vec![format!(
        "cannot remove '{}': Permission denied",
        sealed.display()
    )];
    assert_eq!(dry.errors, expected);
    assert_eq!(real.errors, expected);
    assert!(
        same_stats(&dry.stats, &real.stats),
        "{:?} {:?}",
        dry.stats,
        real.stats
    );
}

// -- -f and ENOTDIR -------------------------------------------------------------

#[cfg(unix)]
#[test]
fn force_ignores_operand_below_a_file() {
    // GNU: `rm -f file.txt/x` is silent and succeeds.
    let tmp = TempDir::new("enotdir");
    let file = tmp.path().join("file.txt");
    write_file(&file, 1);
    let bogus = file.join("x");
    let forced = Options {
        force: true,
        ..Options::default()
    };

    let out = remove_all(std::slice::from_ref(&bogus), &forced);
    assert!(out.is_ok(), "{:?}", out.errors);
    let mut asked = 0;
    let out = remove_all_interactive(std::slice::from_ref(&bogus), &forced, &mut |_| {
        asked += 1;
        true
    });
    assert!(out.is_ok(), "{:?}", out.errors);
    assert_eq!(asked, 0);

    let out = remove_all(std::slice::from_ref(&bogus), &Options::default());
    assert_eq!(
        out.errors,
        vec![format!(
            "cannot remove '{}': Not a directory",
            bogus.display()
        )]
    );
    assert!(file.exists());
}

// -- one file system ------------------------------------------------------------

#[cfg(unix)]
fn skip_msg(p: &Path) -> String {
    format!(
        "skipping '{}', since it's on a different device",
        p.display()
    )
}

#[cfg(unix)]
#[test]
fn one_file_system_skips_directories_on_another_device() {
    // The hook makes every subdirectory look like a mount point.
    let tmp = TempDir::new("xdev-hook");
    let root = tmp.path().join("tree");
    write_file(&root.join("a/f"), 1);
    write_file(&root.join("b/c/g"), 1);
    write_file(&root.join("top.txt"), 1);
    let before = listing(tmp.path());
    let xdev = Options {
        one_file_system: true,
        ..Options::default()
    };

    rmx_lib::__set_pretend_other_device(true);
    let dry = remove_all(
        std::slice::from_ref(&root),
        &Options {
            dry_run: true,
            ..xdev
        },
    );
    let out = remove_all(std::slice::from_ref(&root), &xdev);
    rmx_lib::__set_pretend_other_device(false);

    let mut expected = vec![skip_msg(&root.join("a")), skip_msg(&root.join("b"))];
    expected.sort();
    for (what, mut errors) in [("dry", dry.errors), ("real", out.errors)] {
        errors.sort();
        assert_eq!(errors, expected, "{what}");
    }
    assert_eq!(dry.stats.files_deleted, 1);
    assert_eq!(dry.stats.dirs_deleted, 0);
    // Only the top-level file went; the skipped directories and therefore
    // the operand stay, without a "Directory not empty" error.
    assert!(!exists(&root.join("top.txt")));
    assert!(root.join("a/f").exists());
    assert!(root.join("b/c/g").exists());
    assert_eq!(out.stats.files_deleted, 1);
    assert_eq!(out.stats.dirs_deleted, 0);
    assert_eq!(before.len() - 1, listing(tmp.path()).len());
}

#[cfg(unix)]
#[test]
fn one_file_system_off_ignores_devices() {
    let tmp = TempDir::new("xdev-off");
    let root = tmp.path().join("tree");
    write_file(&root.join("a/f"), 1);
    rmx_lib::__set_pretend_other_device(true);
    let out = remove_all(std::slice::from_ref(&root), &Options::default());
    write_file(&root.join("a/f"), 1);
    let int = remove_all_interactive(
        std::slice::from_ref(&root),
        &Options::default(),
        &mut |_| true,
    );
    rmx_lib::__set_pretend_other_device(false);
    assert!(out.is_ok(), "{:?}", out.errors);
    assert!(int.is_ok(), "{:?}", int.errors);
    assert!(!exists(&root));
}

#[cfg(unix)]
#[test]
fn interactive_one_file_system_skips_other_devices() {
    let tmp = TempDir::new("xdev-int");
    let root = tmp.path().join("tree");
    write_file(&root.join("f"), 1);
    write_file(&root.join("sub/g"), 1);
    let opts = Options {
        one_file_system: true,
        ..Options::default()
    };
    let mut asked = Vec::new();
    rmx_lib::__set_pretend_other_device(true);
    let out = remove_all_interactive(std::slice::from_ref(&root), &opts, &mut |q| {
        asked.push(q.to_string());
        true
    });
    rmx_lib::__set_pretend_other_device(false);
    assert_eq!(out.errors, vec![skip_msg(&root.join("sub"))]);
    assert!(!exists(&root.join("f")));
    assert!(root.join("sub/g").exists());
    assert!(
        !asked.iter().any(|q| q.starts_with("remove directory")),
        "{asked:?}"
    );
}

/// Needs root and a container started with --privileged (or a real host);
/// returns early when tmpfs cannot be mounted.
#[cfg(target_os = "linux")]
#[test]
fn one_file_system_stops_at_a_real_mount_point() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    if !is_root() {
        return;
    }
    let tmp = TempDir::new("xdev-mount");
    let root = tmp.path().join("tree");
    let mnt = root.join("mnt");
    fs::create_dir_all(&mnt).unwrap();
    write_file(&root.join("a/f"), 1);
    let target = CString::new(mnt.as_os_str().as_bytes()).unwrap();
    // SAFETY: valid C strings, no mount data.
    let rc = unsafe {
        libc::mount(
            c"tmpfs".as_ptr(),
            target.as_ptr(),
            c"tmpfs".as_ptr(),
            0,
            std::ptr::null(),
        )
    };
    if rc != 0 {
        eprintln!(
            "skipped: cannot mount tmpfs: {}",
            std::io::Error::last_os_error()
        );
        return;
    }
    struct Unmount(CString);
    impl Drop for Unmount {
        fn drop(&mut self) {
            // SAFETY: valid C string.
            unsafe { libc::umount2(self.0.as_ptr(), libc::MNT_DETACH) };
        }
    }
    let guard = Unmount(target);
    write_file(&mnt.join("inside/g"), 1);
    let xdev = Options {
        one_file_system: true,
        ..Options::default()
    };

    let dry = remove_all(
        std::slice::from_ref(&root),
        &Options {
            dry_run: true,
            ..xdev
        },
    );
    assert_eq!(dry.errors, vec![skip_msg(&mnt)]);
    assert!(root.join("a/f").exists());

    let out = remove_all(std::slice::from_ref(&root), &xdev);
    assert_eq!(out.errors, vec![skip_msg(&mnt)]);
    assert!(mnt.join("inside/g").exists());
    assert!(!exists(&root.join("a")));
    assert!(exists(&root));
    assert!(
        same_stats(&dry.stats, &out.stats),
        "{:?} {:?}",
        dry.stats,
        out.stats
    );
    eprintln!("ran: tmpfs mount point skipped");
    drop(guard);
}

// -- force writable -------------------------------------------------------------

/// Make `p` and every directory below it u+rwx again so cleanup works.
#[cfg(unix)]
fn unlock_tree(p: &Path) {
    let Ok(m) = fs::symlink_metadata(p) else {
        return;
    };
    if !m.is_dir() {
        return;
    }
    set_mode(p, 0o755);
    if let Ok(rd) = fs::read_dir(p) {
        for e in rd.flatten() {
            unlock_tree(&e.path());
        }
    }
}

/// Set every file to `file_mode` and every directory to `dir_mode`,
/// bottom-up.
#[cfg(unix)]
fn lock_tree(p: &Path, dir_mode: u32, file_mode: u32) {
    let m = fs::symlink_metadata(p).unwrap();
    if m.is_dir() {
        for e in fs::read_dir(p).unwrap() {
            lock_tree(&e.unwrap().path(), dir_mode, file_mode);
        }
        set_mode(p, dir_mode);
    } else if m.is_file() {
        set_mode(p, file_mode);
    }
}

/// A Go module cache: everything read-only, plus one unreadable and one
/// unsearchable directory. Returns (files, dirs) including `root`.
#[cfg(unix)]
fn make_mod_cache(root: &Path) -> (usize, usize) {
    for m in 0..40 {
        for d in ["", "internal", "internal/x"] {
            for f in 0..5 {
                let rel = format!("github.com/u/m{m}@v1.0.0/{d}/f{f}.go");
                write_file(&root.join(rel), 10);
            }
        }
    }
    write_file(&root.join("cache/download/sealed/f"), 3);
    write_file(&root.join("cache/download/noexec/f"), 3);
    let all = listing(root);
    let files = all.iter().filter(|e| e.1 == "file").count();
    let dirs = all.iter().filter(|e| e.1 == "dir").count() + 1;
    lock_tree(root, 0o555, 0o444);
    set_mode(&root.join("cache/download/sealed"), 0o000);
    set_mode(&root.join("cache/download/noexec"), 0o600);
    (files, dirs)
}

#[cfg(unix)]
#[test]
fn force_writable_removes_a_read_only_module_cache() {
    for jobs in [0, 1, 3] {
        let tmp = TempDir::new("w-modcache");
        let root = tmp.path().join("mod");
        let (files, dirs) = make_mod_cache(&root);
        let opts = Options {
            force_writable: true,
            jobs,
            ..Options::default()
        };
        let out = remove_all(std::slice::from_ref(&root), &opts);
        unlock_tree(&root);
        assert!(out.is_ok(), "jobs={jobs}: {:?}", out.errors);
        assert!(!exists(&root), "jobs={jobs}");
        assert_eq!(out.stats.files_deleted, files, "jobs={jobs}");
        assert_eq!(out.stats.dirs_deleted, dirs, "jobs={jobs}");
    }
}

#[cfg(unix)]
#[test]
fn read_only_module_cache_fails_without_force_writable() {
    if is_root() {
        return; // root ignores directory permissions
    }
    let tmp = TempDir::new("w-modcache-off");
    let root = tmp.path().join("mod");
    make_mod_cache(&root);
    let out = remove_all(std::slice::from_ref(&root), &Options::default());
    unlock_tree(&root);
    assert!(!out.errors.is_empty());
    assert!(
        out.errors
            .iter()
            .all(|e| e.ends_with(": Permission denied")),
        "{:?}",
        out.errors
    );
    let sealed = root.join("cache/download/sealed");
    assert!(out.errors.contains(&format!(
        "cannot remove '{}': Permission denied",
        sealed.display()
    )));
    assert!(exists(&root));
    assert_eq!(out.stats.files_deleted, 0);

    // A dry run never chmods, so it cannot see inside the sealed directory.
    let again = tmp.path().join("again");
    make_mod_cache(&again);
    let before = listing(&again);
    let dry = remove_all(
        std::slice::from_ref(&again),
        &Options {
            dry_run: true,
            force_writable: true,
            ..Options::default()
        },
    );
    let after = listing(&again);
    unlock_tree(&again);
    assert_eq!(before, after);
    assert_eq!(
        dry.errors,
        vec![format!(
            "cannot remove '{}': Permission denied",
            again.join("cache/download/sealed").display()
        )]
    );
}

#[cfg(unix)]
#[test]
fn force_writable_handles_operands_in_read_only_directories() {
    let tmp = TempDir::new("w-operands");
    let ro = tmp.path().join("ro");
    write_file(&ro.join("a.txt"), 1);
    write_file(&ro.join("sub/b.txt"), 1);
    set_mode(&ro, 0o555);
    let paths = vec![ro.join("a.txt"), ro.join("sub")];

    if !is_root() {
        let out = remove_all(&paths, &Options::default());
        assert_eq!(out.errors.len(), 2, "{:?}", out.errors);
        assert!(exists(&ro.join("a.txt")));
    }
    let opts = Options {
        force_writable: true,
        ..Options::default()
    };
    let out = remove_all(&paths, &opts);
    set_mode(&ro, 0o755);
    assert!(out.is_ok(), "{:?}", out.errors);
    assert_eq!(fs::read_dir(&ro).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn interactive_force_writable_removes_a_read_only_tree() {
    let tmp = TempDir::new("w-int");
    let root = tmp.path().join("mod");
    let (files, dirs) = make_mod_cache(&root);
    let opts = Options {
        force_writable: true,
        ..Options::default()
    };
    let out = remove_all_interactive(std::slice::from_ref(&root), &opts, &mut |_| true);
    unlock_tree(&root);
    assert!(out.is_ok(), "{:?}", out.errors);
    assert!(!exists(&root));
    assert_eq!(out.stats.files_deleted, files);
    assert_eq!(out.stats.dirs_deleted, dirs);
}
