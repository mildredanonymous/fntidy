#[cfg(windows)]
mod windows_rename;

mod check;
mod directory;
mod identity;
#[cfg(any(windows, test))]
mod journal;
mod plan;
mod profile;
mod windows_names;

use std::{env, path::Path, process::ExitCode};

use profile::Profile;

#[cfg(windows)]
const APPLY_HELP: &str = "\n       fntidy apply <file>";
#[cfg(not(windows))]
const APPLY_HELP: &str = "";

const HELP: &str = "Usage: fntidy [--help | --version]\n       fntidy check <directory> [--recursive] [--profile windows|tidy]\n       fntidy plan <directory> [--recursive] [--profile windows|tidy] [--out <file>]\n       fntidy verify-plan <file>";

fn main() -> ExitCode {
    let mut arguments = env::args_os().skip(1);

    match (arguments.next().as_deref(), arguments.next()) {
        (None, None) => println!("{HELP}{APPLY_HELP}"),
        (Some(option), None) if option == "--help" || option == "-h" => {
            println!("{HELP}{APPLY_HELP}")
        }
        (Some(option), None) if option == "--version" || option == "-V" => {
            println!("fntidy {}", env!("CARGO_PKG_VERSION"));
        }
        (Some(command), Some(directory)) if command == "check" || command == "plan" => {
            let mut profile = None;
            let mut output = None;
            let mut recursive = false;
            while let Some(option) = arguments.next() {
                if option == "--recursive" && !recursive {
                    recursive = true;
                    continue;
                }
                let Some(value) = arguments.next() else {
                    return invalid_arguments();
                };
                if option == "--profile" && profile.is_none() {
                    let Some(selected) = Profile::parse(&value) else {
                        return invalid_arguments();
                    };
                    profile = Some(selected);
                } else if option == "--out" && command == "plan" && output.is_none() {
                    output = Some(value);
                } else {
                    return invalid_arguments();
                }
            }
            let root = Path::new(&directory);
            let profile = profile.unwrap_or_default();
            return if command == "check" {
                check::run(root, profile, recursive)
            } else {
                plan::run(root, output.as_deref().map(Path::new), profile, recursive)
            };
        }
        (Some(command), Some(path)) if command == "verify-plan" && arguments.next().is_none() => {
            return plan::verify(Path::new(&path));
        }
        #[cfg(windows)]
        (Some(command), Some(path)) if command == "apply" && arguments.next().is_none() => {
            return plan::apply(Path::new(&path));
        }
        _ => return invalid_arguments(),
    }

    ExitCode::SUCCESS
}

fn invalid_arguments() -> ExitCode {
    eprintln!("Invalid arguments. Run fntidy --help for usage.");
    ExitCode::from(2)
}
