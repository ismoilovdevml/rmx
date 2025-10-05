mod args;
mod commands;
mod upgrade;
mod util;

fn main() {
    let raw_args = args::parse_args();

    // Check for special commands first
    if raw_args.len() >= 2 {
        let first_arg = &raw_args[1];

        match first_arg.as_str() {
            "--version" | "-v" => {
                commands::print_version();
                return;
            }
            "--help" | "-h" | "help" => {
                commands::print_help();
                return;
            }
            "version" => {
                commands::print_version();
                return;
            }
            "about" => {
                commands::print_about();
                return;
            }
            "dev" => {
                commands::print_dev();
                return;
            }
            "upgrade" => {
                if let Err(e) = upgrade::upgrade() {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
                return;
            }
            "check-update" => {
                match upgrade::check_for_updates() {
                    Ok(Some(_)) => {
                        println!("Run 'rmx upgrade' to update");
                    }
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    }
                }
                return;
            }
            _ => {}
        }
    }

    // Parse flags and paths
    match args::parse_flags(&raw_args) {
        Some(parsed_args) => {
            commands::execute_removal(&parsed_args);
        }
        None => {
            commands::print_help();
            std::process::exit(1);
        }
    }
}
