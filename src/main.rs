mod args;
mod commands;
mod style;
mod upgrade;
mod util;

use args::Command;

fn main() {
    let argv: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();

    let command = match args::parse(argv) {
        Ok(c) => c,
        Err(msg) => {
            eprintln!(
                "{}",
                style::paint(style::Stream::Stderr, style::RED, &format!("rmx: {msg}"))
            );
            eprintln!("Try 'rmx --help' for more information.");
            std::process::exit(1);
        }
    };

    match command {
        Command::Remove(a) => std::process::exit(commands::execute_removal(&a)),
        Command::Help => commands::print_help(),
        Command::Version => commands::print_version(),
        Command::About => commands::print_about(),
        Command::Dev => commands::print_dev(),
        Command::Upgrade => {
            if let Err(e) = upgrade::upgrade() {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
        Command::CheckUpdate => match upgrade::check_for_updates() {
            Ok(Some(_)) => println!("Run 'rmx upgrade' to update"),
            Ok(None) => {}
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        },
    }
}
