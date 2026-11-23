mod check;
mod windows_names;

use std::{env, path::Path, process::ExitCode};

const HELP: &str = "Usage: fntidy [--help | --version]\n       fntidy check <directory>";

fn main() -> ExitCode {
    let mut arguments = env::args_os().skip(1);

    match (arguments.next().as_deref(), arguments.next()) {
        (None, None) => println!("{HELP}"),
        (Some(option), None) if option == "--help" || option == "-h" => println!("{HELP}"),
        (Some(option), None) if option == "--version" || option == "-V" => {
            println!("fntidy {}", env!("CARGO_PKG_VERSION"));
        }
        (Some(command), Some(directory)) if command == "check" && arguments.next().is_none() => {
            return check::run(Path::new(&directory));
        }
        _ => {
            eprintln!("Invalid arguments. Run fntidy --help for usage.");
            return ExitCode::from(2);
        }
    }

    ExitCode::SUCCESS
}
