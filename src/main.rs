mod args;
mod commands;
mod util;

fn main() {
    let args = args::parse_args();

    if args.len() < 2 {
        eprintln!("The path to the folder is not correct");
        std::process::exit(-1);
    }

    let command = &args[1];
    let path_str = args.get(2).map(|s| s.as_str()); // Convert &String to Option<&str>

    commands::execute_command(command, path_str);
}
