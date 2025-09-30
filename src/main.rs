mod args;
mod commands;
mod util;

fn main() {
    let args = args::parse_args();

    // rmx binary nomi args[0] da
    if args.len() < 2 {
        commands::print_help();
        std::process::exit(1);
    }

    let first_arg = &args[1];

    // Check for flags and special commands
    match first_arg.as_str() {
        "--version" | "-v" => {
            commands::print_version();
        }
        "--help" | "-h" | "help" => {
            commands::print_help();
        }
        "version" => {
            commands::print_version();
        }
        "about" => {
            commands::print_about();
        }
        "dev" => {
            commands::print_dev();
        }
        _ => {
            // Default: treat first argument as path
            commands::delete_directory(first_arg);
        }
    }
}
