mod args;
mod commands;
mod util;

fn main() {
    let args = args::parse_args();

    if args.len() < 2 {
        eprintln!("The program name is not provided");
        std::process::exit(-1);
    }

    let command = &args[1];
    if command != "rmx" {
        eprintln!("Invalid command: expected 'rmx'");
        std::process::exit(-1);
    }

    for arg in args.iter().skip(2) {
        let command_str = arg.strip_prefix('-');
        commands::execute_command(command_str);
    }
}
