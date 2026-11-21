//! End-to-end tests: run the real `rmx` binary and check files, exit codes and
//! output. These pin down the rm-compatibility and safety behaviour.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "rmx-cli-{}-{}-{}",
            tag,
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        TempDir(path.canonicalize().unwrap())
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn file(&self, rel: &str) -> PathBuf {
        let p = self.0.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&p, b"data").unwrap();
        p
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        restore_perms(&self.0);
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(not(unix))]
fn restore_perms(_: &Path) {}

#[cfg(unix)]
fn restore_perms(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(entries) = fs::read_dir(p) {
        for e in entries.flatten() {
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let _ = fs::set_permissions(e.path(), fs::Permissions::from_mode(0o755));
                restore_perms(&e.path());
            }
        }
    }
}

fn rmx_in(dir: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rmx"))
        .args(args)
        .current_dir(dir)
        .env_remove("NO_COLOR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // rmx may exit without reading stdin (e.g. an operand it refuses), so a
    // broken pipe here is expected and not a test failure.
    let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
    child.wait_with_output().unwrap()
}

fn rmx(dir: &Path, args: &[&str]) -> Output {
    rmx_in(dir, args, "")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[cfg(unix)]
fn is_root_user() -> bool {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() == 0 }
}

#[test]
fn lowercase_v_is_verbose_and_deletes() {
    let t = TempDir::new("v");
    let f = t.file("file.txt");
    let o = rmx(t.path(), &["-v", "file.txt"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(!f.exists());
    assert!(stdout(&o).contains("removed 'file.txt'"), "{}", stdout(&o));
}

#[test]
fn verbose_recursive_example_from_help_works() {
    let t = TempDir::new("vr");
    t.file("build/a/b.txt");
    let o = rmx(t.path(), &["-v", "-r", "build/"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(!t.path().join("build").exists());
    assert!(stdout(&o).contains("removed directory"));
}

#[test]
fn version_flags() {
    let t = TempDir::new("ver");
    for flag in ["-V", "--version", "version"] {
        let o = rmx(t.path(), &[flag]);
        assert!(o.status.success());
        assert_eq!(
            stdout(&o).trim(),
            format!("rmx v{}", env!("CARGO_PKG_VERSION"))
        );
    }
}

#[test]
fn piped_output_has_no_ansi_escapes() {
    let t = TempDir::new("ansi");
    t.file("a");
    for args in [
        &["version"][..],
        &["--help"][..],
        &["a"][..],
        &["missing"][..],
    ] {
        let o = rmx(t.path(), args);
        assert!(!stdout(&o).contains('\x1b'), "{args:?}: {:?}", stdout(&o));
        assert!(!stderr(&o).contains('\x1b'), "{args:?}: {:?}", stderr(&o));
    }
}

#[test]
fn double_dash_and_lone_dash() {
    let t = TempDir::new("dash");
    let weird = t.file("-weird.txt");
    let dash = t.file("-");
    let o = rmx(t.path(), &["--", "-weird.txt", "-"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(!weird.exists());
    assert!(!dash.exists());
}

#[test]
fn unknown_long_option_deletes_nothing_even_with_force() {
    let t = TempDir::new("unk");
    let f = t.file("file.txt");
    let o = rmx(t.path(), &["-f", "--recrusive", "file.txt"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(f.exists());
    assert!(stderr(&o).contains("unrecognized option '--recrusive'"));
}

#[test]
fn force_after_interactive_does_not_prompt() {
    let t = TempDir::new("if");
    let f = t.file("x");
    let o = rmx_in(t.path(), &["-if", "x"], "n\n");
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(!f.exists());
    assert!(!stderr(&o).contains('?'));
}

#[test]
fn interactive_after_force_prompts() {
    let t = TempDir::new("fi");
    let f = t.file("x");
    let o = rmx_in(t.path(), &["-fi", "x"], "n\n");
    assert!(o.status.success());
    assert!(f.exists());
    assert!(stderr(&o).contains("remove regular file 'x'?"));
}

#[test]
fn interactive_recursive_asks_per_entry() {
    let t = TempDir::new("ri");
    t.file("d/keep.txt");
    let o = rmx_in(t.path(), &["-ri", "d"], "y\nn\ny\n");
    let err = stderr(&o);
    assert!(err.contains("descend into directory 'd'?"), "{err}");
    let sep = std::path::MAIN_SEPARATOR;
    assert!(
        err.contains(&format!("remove regular file 'd{sep}keep.txt'?")),
        "{err}"
    );
    // Declined the file, so it and its directory survive.
    assert!(t.path().join("d/keep.txt").exists());
}

#[test]
fn interactive_recursive_yes_to_all() {
    let t = TempDir::new("riy");
    t.file("d/a.txt");
    t.file("d/sub/b.txt");
    let o = rmx_in(t.path(), &["-ri", "d"], &"y\n".repeat(20));
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(!t.path().join("d").exists());
}

#[cfg(unix)]
#[test]
fn force_still_reports_permission_errors() {
    use std::os::unix::fs::PermissionsExt;
    if is_root_user() {
        return;
    }
    let t = TempDir::new("perm");
    t.file("locked/f");
    let free = t.file("free.txt");
    fs::set_permissions(t.path().join("locked"), fs::Permissions::from_mode(0o555)).unwrap();

    let o = rmx(t.path(), &["-rf", "locked", "free.txt"]);
    assert_eq!(o.status.code(), Some(1), "must not exit 0 on EACCES");
    assert!(stderr(&o).contains("Permission denied"), "{}", stderr(&o));
    assert!(t.path().join("locked/f").exists());
    // The failure did not stop the other operand.
    assert!(!free.exists());
}

#[test]
fn missing_operand_does_not_stop_the_rest() {
    let t = TempDir::new("miss");
    let a = t.file("a.txt");
    let b = t.file("b.txt");
    let o = rmx(t.path(), &["missing.txt", "a.txt", "b.txt"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("cannot remove 'missing.txt': No such file or directory"));
    assert!(!a.exists() && !b.exists());
}

#[test]
fn force_ignores_missing_silently() {
    let t = TempDir::new("fmiss");
    let o = rmx(t.path(), &["-f", "missing.txt"]);
    assert!(o.status.success());
    assert!(stderr(&o).is_empty(), "{}", stderr(&o));
    // rm -f with no operands is fine too.
    assert!(rmx(t.path(), &["-f"]).status.success());
}

#[test]
fn directory_without_recursive_is_an_error() {
    let t = TempDir::new("isdir");
    t.file("d/f");
    let o = rmx(t.path(), &["d"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("Is a directory"));
    assert!(t.path().join("d/f").exists());
}

#[test]
fn dir_flag_removes_only_empty_directories() {
    let t = TempDir::new("dflag");
    fs::create_dir(t.path().join("empty")).unwrap();
    t.file("full/f");
    let o = rmx(t.path(), &["-d", "empty", "full"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(!t.path().join("empty").exists());
    assert!(t.path().join("full/f").exists());
}

#[test]
fn refuses_dot_and_dotdot() {
    let t = TempDir::new("dots");
    t.file("d/f");
    for arg in [".", "./", "..", "d/.", "d/.."] {
        let o = rmx(t.path(), &["-rf", arg]);
        assert_eq!(o.status.code(), Some(1), "{arg}");
        assert!(
            stderr(&o).contains("refusing to remove '.' or '..'"),
            "{arg}"
        );
    }
    assert!(t.path().join("d/f").exists());
}

#[test]
fn preserve_root() {
    let t = TempDir::new("root");
    // -i with "n" answers is a belt-and-braces guard: even if the check
    // regressed, nothing would be removed.
    for arg in ["/", "//", "/../"] {
        let o = rmx_in(t.path(), &["-ri", arg], &"n\n".repeat(10));
        assert_eq!(o.status.code(), Some(1), "{arg}");
        let err = stderr(&o);
        assert!(
            err.contains("dangerous to operate recursively on")
                || err.contains("refusing to remove '.' or '..'"),
            "{arg}: {err}"
        );
        assert!(!err.contains("descend into"), "{arg}: prompted: {err}");
    }
}

#[cfg(unix)]
#[test]
fn symlink_to_dir_with_trailing_slash_is_refused() {
    let t = TempDir::new("slash");
    let precious = t.file("target/precious.txt");
    std::os::unix::fs::symlink(t.path().join("target"), t.path().join("link")).unwrap();

    let o = rmx(t.path(), &["-rf", "link/"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        precious.exists(),
        "followed the symlink and deleted target data"
    );
    assert!(fs::symlink_metadata(t.path().join("link")).is_ok());

    // Without the slash the link itself goes, the target stays.
    let o = rmx(t.path(), &["-rf", "link"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(fs::symlink_metadata(t.path().join("link")).is_err());
    assert!(precious.exists());
}

#[test]
fn real_directory_with_trailing_slash_is_removed() {
    let t = TempDir::new("dirslash");
    t.file("d/f");
    let o = rmx(t.path(), &["-rf", "d/"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(!t.path().join("d").exists());
}

#[test]
fn files_named_like_subcommands_are_removable() {
    let t = TempDir::new("sub");
    let dev = t.file("dev");
    let about = t.file("about");
    let version = t.file("version");
    assert!(rmx(t.path(), &["dev", "about"]).status.success());
    assert!(!dev.exists() && !about.exists());
    assert!(rmx(t.path(), &["--", "version"]).status.success());
    assert!(!version.exists());
}

#[test]
fn quiet_prints_nothing() {
    let t = TempDir::new("quiet");
    t.file("d/f");
    let o = rmx(t.path(), &["-rfq", "d"]);
    assert!(o.status.success());
    assert!(stdout(&o).is_empty());
    assert!(stderr(&o).is_empty());
}

#[test]
fn summary_counts() {
    let t = TempDir::new("sum");
    t.file("d/a");
    t.file("d/s/b");
    let o = rmx(t.path(), &["-rf", "d"]);
    assert!(o.status.success());
    assert!(
        stdout(&o).contains("Deleted: 2 files, 2 directories"),
        "{}",
        stdout(&o)
    );
}

#[test]
fn many_top_level_files() {
    let t = TempDir::new("many");
    let names: Vec<String> = (0..2000).map(|i| format!("f{i}.log")).collect();
    for n in &names {
        t.file(n);
    }
    let mut args = vec!["-f"];
    args.extend(names.iter().map(String::as_str));
    let o = rmx(t.path(), &args);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(fs::read_dir(t.path()).unwrap().count(), 0);
}

#[test]
fn jobs_flag() {
    let t = TempDir::new("jobs");
    for i in 0..600 {
        t.file(&format!("d/{}/f{i}", i % 7));
    }
    for j in ["1", "2", "8"] {
        if !t.path().join("d").exists() {
            for i in 0..600 {
                t.file(&format!("d/{}/f{i}", i % 7));
            }
        }
        let o = rmx(t.path(), &["-rfq", "-j", j, "d"]);
        assert!(o.status.success(), "-j {j}: {}", stderr(&o));
        assert!(!t.path().join("d").exists());
    }
}

#[cfg(unix)]
#[test]
fn non_utf8_names() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let t = TempDir::new("nonutf8");
    let name = OsStr::from_bytes(b"bad\xffname");
    let top = t.path().join(name);
    if fs::write(&top, b"x").is_err() {
        // Filesystems that enforce UTF-8 names (APFS can) — nothing to test.
        return;
    }
    let o = Command::new(env!("CARGO_BIN_EXE_rmx"))
        .arg("-f")
        .arg(name)
        .current_dir(t.path())
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(!top.exists());
}

fn listing(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            if e.file_type().unwrap().is_dir() {
                stack.push(p.clone());
            }
            out.push(p);
        }
    }
    out.sort();
    out
}

#[test]
fn dry_run_removes_nothing_and_reports_counts() {
    let t = TempDir::new("dry");
    for i in 0..300 {
        t.file(&format!("d/{}/f{i}", i % 9));
    }
    let before = listing(t.path());
    let o = rmx(t.path(), &["-rn", "d"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(listing(t.path()), before, "dry run changed the tree");
    let out = stdout(&o);
    assert!(
        out.contains("Would delete: 300 files, 10 directories"),
        "{out}"
    );

    let o = rmx(t.path(), &["-rnv", "d"]);
    let sep = std::path::MAIN_SEPARATOR;
    assert!(
        stdout(&o).contains(&format!("would remove 'd{sep}")),
        "{}",
        stdout(&o)
    );
    assert_eq!(listing(t.path()), before);
}

#[test]
fn async_returns_and_deletes_in_background() {
    let t = TempDir::new("async");
    for i in 0..2000 {
        t.file(&format!("big/{}/f{i}", i % 40));
    }
    let keep = t.file("keep.txt");
    let o = rmx(t.path(), &["-rf", "--async", "big"]);
    assert!(o.status.success(), "{}", stderr(&o));
    // The name is free the moment rmx returns.
    assert!(!t.path().join("big").exists());
    assert!(stdout(&o).contains("in the background"), "{}", stdout(&o));

    // The detached child finishes the job; no trash may be left behind.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let leftovers: Vec<_> = fs::read_dir(t.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(".rmx-trash-"))
            .collect();
        if leftovers.is_empty() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "background deletion did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(keep.exists());
}

#[test]
fn async_with_files_deletes_them_directly() {
    let t = TempDir::new("asyncf");
    let f = t.file("a.txt");
    let o = rmx(t.path(), &["-rf", "--async", "a.txt"]);
    assert!(o.status.success());
    assert!(!f.exists());
}

#[cfg(unix)]
#[test]
fn force_writable_removes_read_only_trees() {
    use std::os::unix::fs::PermissionsExt;
    if is_root_user() {
        return;
    }
    let t = TempDir::new("ro");
    t.file("mod/pkg@v1/a.go");
    t.file("mod/pkg@v1/sub/b.go");
    for d in ["mod/pkg@v1/sub", "mod/pkg@v1", "mod"] {
        fs::set_permissions(t.path().join(d), fs::Permissions::from_mode(0o555)).unwrap();
    }

    let o = rmx(t.path(), &["-rf", "mod"]);
    assert_eq!(
        o.status.code(),
        Some(1),
        "read-only tree removed without -W"
    );

    let o = rmx(t.path(), &["-rfW", "mod"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(!t.path().join("mod").exists());
}

#[test]
fn no_progress_line_when_stderr_is_not_a_terminal() {
    let t = TempDir::new("prog");
    for i in 0..3000 {
        t.file(&format!("d/{}/f{i}", i % 30));
    }
    let o = rmx(t.path(), &["-rf", "d"]);
    assert!(o.status.success());
    assert!(stderr(&o).is_empty(), "{:?}", stderr(&o));
}
