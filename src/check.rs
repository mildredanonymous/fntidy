use std::{cmp::Ordering, ffi::OsStr, path::Path, process::ExitCode};

use crate::{
    directory::{self, display},
    profile::Profile,
    windows_names::{self, Rule},
};

pub fn run(root: &Path, profile: Profile) -> ExitCode {
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
            for (id, reason) in profile
                .violations(name)
                .chain((group.len() > 1).then_some(Rule::NameCollision.diagnostic()))
            {
                println!(
                    "[{}] {id} {}: {reason}",
                    profile.name(),
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
        match profile {
            Profile::Windows => println!("No Windows naming violations found."),
            Profile::Tidy => println!("No tidy naming violations found."),
        }
        ExitCode::SUCCESS
    }
}
