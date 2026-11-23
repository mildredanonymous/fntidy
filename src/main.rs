use std::{env, process::ExitCode};

const HELP: &str = "Usage: fntidy [--help | --version]";

fn main() -> ExitCode {
    let mut arguments = env::args_os().skip(1);

    match (arguments.next().as_deref(), arguments.next()) {
        (None, None) => println!("{HELP}"),
        (Some(option), None) if option == "--help" || option == "-h" => println!("{HELP}"),
        (Some(option), None) if option == "--version" || option == "-V" => {
            println!("fntidy {}", env!("CARGO_PKG_VERSION"));
        }
        _ => {
            eprintln!("Invalid arguments. Run fntidy --help for usage.");
            return ExitCode::from(2);
        }
    }

    ExitCode::SUCCESS
}
