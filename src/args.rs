use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct RmxArgs {
    pub paths: Vec<PathBuf>,
    pub recursive: bool,      // -r, -R, --recursive
    pub force: bool,          // -f, --force
    pub interactive: bool,    // -i, --interactive
    pub verbose: bool,        // -v, --verbose
    pub dir: bool,            // -d, --dir (remove empty directories)
    pub quiet: bool,          // -q, --quiet (no summary, no byte accounting)
    pub preserve_root: bool,  // --[no-]preserve-root
    pub jobs: usize,          // -j, --jobs (0 = auto)
    pub dry_run: bool,        // -n, --dry-run
    pub one_fs: bool,         // -x, --one-file-system
    pub force_writable: bool, // -W, --force-writable
    pub background: bool,     // --async
}

impl Default for RmxArgs {
    fn default() -> Self {
        RmxArgs {
            paths: Vec::new(),
            recursive: false,
            force: false,
            interactive: false,
            verbose: false,
            dir: false,
            quiet: false,
            preserve_root: true,
            jobs: 0,
            dry_run: false,
            one_fs: false,
            force_writable: false,
            background: false,
        }
    }
}

#[derive(Debug)]
pub enum Command {
    Remove(RmxArgs),
    Help,
    Version,
    About,
    Dev,
    Upgrade,
    CheckUpdate,
}

/// Parse the command line (without argv[0]).
///
/// Subcommands (`version`, `upgrade`, ...) are only recognised when they are
/// the sole argument, so `rmx -r dev` or `rmx dev other` still remove files
/// named `dev`. To remove a lone file called `version`, use `rmx -- version`
/// or `rmx ./version`.
pub fn parse(args: Vec<OsString>) -> Result<Command, String> {
    if args.len() == 1 {
        let sub = match args[0].to_str() {
            Some("help") => Some(Command::Help),
            Some("version") => Some(Command::Version),
            Some("about") => Some(Command::About),
            Some("dev") => Some(Command::Dev),
            Some("upgrade") => Some(Command::Upgrade),
            Some("check-update") => Some(Command::CheckUpdate),
            _ => None,
        };
        if let Some(sub) = sub {
            return Ok(sub);
        }
    }

    let mut out = RmxArgs::default();
    let mut only_paths = false;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        if only_paths {
            out.paths.push(PathBuf::from(arg));
            continue;
        }

        let bytes = arg.as_encoded_bytes();
        // "-" on its own is a file name, as is anything not starting with '-'.
        if bytes.len() < 2 || bytes[0] != b'-' {
            out.paths.push(PathBuf::from(arg));
            continue;
        }
        if bytes == b"--" {
            only_paths = true;
            continue;
        }

        // Options are ASCII; a non-UTF-8 argument starting with '-' is not one.
        let s = arg
            .to_str()
            .ok_or_else(|| format!("invalid option '{}'", arg.to_string_lossy()))?;

        if let Some(long) = s.strip_prefix("--") {
            let (name, value) = match long.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (long, None),
            };
            match name {
                "recursive" => out.recursive = true,
                "force" => set_force(&mut out),
                "interactive" => set_interactive(&mut out),
                "verbose" => out.verbose = true,
                "dir" => out.dir = true,
                "quiet" => out.quiet = true,
                "preserve-root" => out.preserve_root = true,
                "no-preserve-root" => out.preserve_root = false,
                "dry-run" => out.dry_run = true,
                "one-file-system" => out.one_fs = true,
                "force-writable" => out.force_writable = true,
                "async" => out.background = true,
                "help" => return Ok(Command::Help),
                "version" => return Ok(Command::Version),
                "jobs" => {
                    let v = match value {
                        Some(v) => v,
                        None => next_value(&mut iter, "--jobs")?,
                    };
                    out.jobs = parse_jobs(&v)?;
                    continue;
                }
                _ => return Err(format!("unrecognized option '--{name}'")),
            }
            if value.is_some() {
                return Err(format!("option '--{name}' doesn't allow an argument"));
            }
            continue;
        }

        // Short options, combinable: -rf, -rfv, -j4, -j 4.
        let chars = s[1..].char_indices();
        for (idx, ch) in chars {
            match ch {
                'r' | 'R' => out.recursive = true,
                'f' => set_force(&mut out),
                'i' => set_interactive(&mut out),
                'v' => out.verbose = true,
                'd' => out.dir = true,
                'q' => out.quiet = true,
                'n' => out.dry_run = true,
                'x' => out.one_fs = true,
                'W' => out.force_writable = true,
                'h' => return Ok(Command::Help),
                'V' => return Ok(Command::Version),
                'j' => {
                    let rest = &s[1 + idx + 1..];
                    let v = if rest.is_empty() {
                        next_value(&mut iter, "-j")?
                    } else {
                        rest.to_string()
                    };
                    out.jobs = parse_jobs(&v)?;
                    break;
                }
                _ => return Err(format!("invalid option -- '{ch}'")),
            }
        }
    }

    if out.paths.is_empty() && !out.force {
        return Err("missing operand".to_string());
    }
    if out.background && (out.interactive || out.dry_run) {
        return Err("--async cannot be combined with -i or --dry-run".to_string());
    }

    Ok(Command::Remove(out))
}

/// POSIX: the last of -f / -i wins.
fn set_force(a: &mut RmxArgs) {
    a.force = true;
    a.interactive = false;
}

fn set_interactive(a: &mut RmxArgs) {
    a.interactive = true;
    a.force = false;
}

fn next_value(iter: &mut impl Iterator<Item = OsString>, flag: &str) -> Result<String, String> {
    iter.next()
        .and_then(|v| v.into_string().ok())
        .ok_or_else(|| format!("option '{flag}' requires an argument"))
}

fn parse_jobs(v: &str) -> Result<usize, String> {
    match v.parse::<usize>() {
        Ok(n) if (1..=256).contains(&n) => Ok(n),
        _ => Err(format!("invalid number of jobs: '{v}' (expected 1-256)")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Command, String> {
        parse(args.iter().map(OsString::from).collect())
    }

    fn remove(args: &[&str]) -> RmxArgs {
        match p(args) {
            Ok(Command::Remove(a)) => a,
            other => panic!("expected Remove, got {other:?}"),
        }
    }

    #[test]
    fn lowercase_v_is_verbose_not_version() {
        let a = remove(&["-v", "file.txt"]);
        assert!(a.verbose);
        assert_eq!(a.paths, vec![PathBuf::from("file.txt")]);
        assert!(matches!(p(&["-V"]), Ok(Command::Version)));
        assert!(matches!(p(&["--version"]), Ok(Command::Version)));
    }

    #[test]
    fn double_dash_ends_options() {
        let a = remove(&["-f", "--", "-weird.txt", "--force"]);
        assert!(a.force);
        assert_eq!(
            a.paths,
            vec![PathBuf::from("-weird.txt"), PathBuf::from("--force")]
        );
    }

    #[test]
    fn lone_dash_is_a_path() {
        assert_eq!(remove(&["-"]).paths, vec![PathBuf::from("-")]);
    }

    #[test]
    fn unknown_long_option_is_rejected_even_with_force() {
        assert!(p(&["-f", "--recrusive", "file.txt"]).is_err());
        assert!(p(&["-z", "file.txt"]).is_err());
    }

    #[test]
    fn new_flags() {
        let a = remove(&["-rfnxW", "d"]);
        assert!(a.dry_run && a.one_fs && a.force_writable);
        let a = remove(&[
            "--dry-run",
            "--one-file-system",
            "--force-writable",
            "-r",
            "d",
        ]);
        assert!(a.dry_run && a.one_fs && a.force_writable);
        assert!(remove(&["--async", "-rf", "d"]).background);
        assert!(p(&["--async", "-ri", "d"]).is_err());
        assert!(p(&["--async", "-rn", "d"]).is_err());
    }

    #[test]
    fn last_of_force_and_interactive_wins() {
        let a = remove(&["-if", "x"]);
        assert!(a.force && !a.interactive);
        let a = remove(&["-fi", "x"]);
        assert!(a.interactive && !a.force);
        let a = remove(&["-i", "--force", "x"]);
        assert!(a.force && !a.interactive);
    }

    #[test]
    fn subcommands_only_when_sole_argument() {
        assert!(matches!(p(&["version"]), Ok(Command::Version)));
        assert!(matches!(p(&["upgrade"]), Ok(Command::Upgrade)));
        assert_eq!(
            remove(&["dev", "about"]).paths,
            vec![PathBuf::from("dev"), PathBuf::from("about")]
        );
        assert_eq!(
            remove(&["--", "version"]).paths,
            vec![PathBuf::from("version")]
        );
    }

    #[test]
    fn missing_operand() {
        assert!(p(&[]).is_err());
        assert!(p(&["-r"]).is_err());
        // rm -f with no operands is not an error.
        assert!(remove(&["-f"]).paths.is_empty());
    }

    #[test]
    fn jobs_forms() {
        assert_eq!(remove(&["-j", "2", "x"]).jobs, 2);
        assert_eq!(remove(&["-rfj3", "x"]).jobs, 3);
        assert_eq!(remove(&["--jobs=6", "x"]).jobs, 6);
        assert_eq!(remove(&["--jobs", "1", "x"]).jobs, 1);
        assert!(p(&["-j", "0", "x"]).is_err());
        assert!(p(&["--jobs=abc", "x"]).is_err());
        assert!(p(&["-j"]).is_err());
    }

    #[test]
    fn preserve_root_default_and_override() {
        assert!(remove(&["-rf", "x"]).preserve_root);
        assert!(!remove(&["-rf", "--no-preserve-root", "x"]).preserve_root);
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_path_is_accepted() {
        use std::os::unix::ffi::OsStringExt;
        let name = OsString::from_vec(vec![b'f', 0xff, b'x']);
        match parse(vec![OsString::from("-f"), name.clone()]) {
            Ok(Command::Remove(a)) => assert_eq!(a.paths, vec![PathBuf::from(name)]),
            other => panic!("{other:?}"),
        }
    }
}
