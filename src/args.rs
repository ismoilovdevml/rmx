use std::env;

#[derive(Debug, Clone)]
pub struct RmxArgs {
    pub paths: Vec<String>,
    pub recursive: bool,      // -r, -R, --recursive
    pub force: bool,          // -f, --force
    pub interactive: bool,    // -i, --interactive
    pub verbose: bool,        // -v, --verbose
    pub dir: bool,            // -d, --dir (remove empty directories)
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
        }
    }
}

pub fn parse_args() -> Vec<String> {
    env::args().collect()
}

pub fn parse_flags(args: &[String]) -> Option<RmxArgs> {
    if args.len() < 2 {
        return None;
    }

    let mut rmx_args = RmxArgs::default();
    let mut i = 1;

    while i < args.len() {
        let arg = &args[i];

        match arg.as_str() {
            // Long flags
            "--recursive" => rmx_args.recursive = true,
            "--force" => rmx_args.force = true,
            "--interactive" => rmx_args.interactive = true,
            "--verbose" => rmx_args.verbose = true,
            "--dir" => rmx_args.dir = true,

            // Short flags (can be combined like -rf)
            s if s.starts_with('-') && !s.starts_with("--") => {
                for ch in s.chars().skip(1) {
                    match ch {
                        'r' | 'R' => rmx_args.recursive = true,
                        'f' => rmx_args.force = true,
                        'i' => rmx_args.interactive = true,
                        'v' => rmx_args.verbose = true,
                        'd' => rmx_args.dir = true,
                        _ => {
                            eprintln!("Unknown flag: -{}", ch);
                            return None;
                        }
                    }
                }
            }

            // Paths
            _ => {
                rmx_args.paths.push(arg.clone());
            }
        }

        i += 1;
    }

    if rmx_args.paths.is_empty() {
        return None;
    }

    Some(rmx_args)
}
