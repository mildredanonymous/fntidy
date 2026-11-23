use std::{cmp::Ordering, ffi::OsStr, fs, path::Path, process::ExitCode};

use crate::windows_names::{self, Rule};

pub fn run(root: &Path) -> ExitCode {
    let root = root.components().collect::<std::path::PathBuf>();
    let root = root.as_path();
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) => {
            eprintln!("Cannot inspect {}: {error}", display(root.as_os_str()));
            return ExitCode::from(2);
        }
    };
    if redirected(&metadata) || !metadata.is_dir() {
        eprintln!(
            "Expected a directory without redirection: {}",
            display(root.as_os_str())
        );
        return ExitCode::from(2);
    }
    let directory = match fs::read_dir(root) {
        Ok(directory) => directory,
        Err(error) => {
            eprintln!("Cannot read {}: {error}", display(root.as_os_str()));
            return ExitCode::from(2);
        }
    };

    let mut entries = Vec::new();
    let mut incomplete = false;
    for item in directory {
        let entry = match item {
            Ok(entry) => entry,
            Err(error) => {
                eprintln!("Cannot read an entry: {error}");
                incomplete = true;
                continue;
            }
        };
        let native_name = entry.file_name();
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                eprintln!("Cannot inspect {}: {error}", display(&native_name));
                incomplete = true;
                continue;
            }
        };
        let Some(name) = native_name.into_string().ok() else {
            eprintln!("Skipped an entry whose name cannot be represented as Unicode.");
            incomplete = true;
            continue;
        };
        // Redirected entries still occupy a name in the same directory.
        entries.push((
            windows_names::comparison_units(&name),
            name,
            redirected(&metadata),
        ));
    }

    entries.sort_unstable_by(|left, right| {
        windows_names::compare(&left.0, &right.0).then_with(|| left.1.cmp(&right.1))
    });
    let mut violations = false;
    for group in
        entries.chunk_by(|left, right| windows_names::compare(&left.0, &right.0) == Ordering::Equal)
    {
        for (_, name, is_redirected) in group {
            if *is_redirected {
                eprintln!("Skipped redirected entry {}.", display(OsStr::new(name)));
                continue;
            }
            for rule in windows_names::violations(name)
                .chain((group.len() > 1).then_some(Rule::NameCollision))
            {
                let (id, reason) = rule.diagnostic();
                println!(
                    "[{}] {id} {}: {reason}",
                    windows_names::PROFILE,
                    display(OsStr::new(name))
                );
                violations = true;
            }
        }
    }

    if incomplete {
        eprintln!("The directory check was incomplete.");
        ExitCode::from(2)
    } else if violations {
        ExitCode::from(1)
    } else {
        println!("No Windows naming violations found.");
        ExitCode::SUCCESS
    }
}

fn redirected(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    metadata.file_type().is_symlink()
}

fn display(value: &OsStr) -> String {
    format!("{value:?}")
}
