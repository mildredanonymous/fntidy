use std::{cmp::Ordering, ffi::OsStr, path::Path, process::ExitCode};

use crate::{
    directory::{self, display},
    windows_names::{self, Rule},
};

pub fn run(root: &Path) -> ExitCode {
    let Some(directory) = directory::scan(root) else {
        return ExitCode::from(2);
    };
    let mut violations = false;
    for group in directory
        .entries
        .chunk_by(|left, right| windows_names::compare(&left.key, &right.key) == Ordering::Equal)
    {
        for entry in group {
            let name = &entry.name;
            if entry.redirected {
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

    if directory.incomplete {
        eprintln!("The directory check was incomplete.");
        ExitCode::from(2)
    } else if violations {
        ExitCode::from(1)
    } else {
        println!("No Windows naming violations found.");
        ExitCode::SUCCESS
    }
}
