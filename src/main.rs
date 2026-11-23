#[cfg(all(windows, test))]
mod windows_rename;

mod check;
mod directory;
mod identity;
mod plan;
mod windows_names;

use std::{env, path::Path, process::ExitCode};

const HELP: &str = "Usage: fntidy [--help | --version]\n       fntidy check <directory>\n       fntidy plan <directory> [--out <file>]\n       fntidy verify-plan <file>";

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
        (Some(command), Some(directory)) if command == "plan" => {
            let output = match (arguments.next().as_deref(), arguments.next()) {
                (None, None) => None,
                (Some(option), Some(path)) if option == "--out" && arguments.next().is_none() => {
                    Some(path)
                }
                _ => return invalid_arguments(),
            };
            return plan::run(Path::new(&directory), output.as_deref().map(Path::new));
        }
        (Some(command), Some(path)) if command == "verify-plan" && arguments.next().is_none() => {
            return plan::verify(Path::new(&path));
        }
        _ => return invalid_arguments(),
    }

    ExitCode::SUCCESS
}

fn invalid_arguments() -> ExitCode {
    eprintln!("Invalid arguments. Run fntidy --help for usage.");
    ExitCode::from(2)
}
