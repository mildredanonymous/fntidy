use std::{cmp::Ordering, path::Path, process::ExitCode};

use crate::{
    directory::{self, display},
    profile::Profile,
    windows_names::{self, Rule},
};

pub fn run(root: &Path, profile: Profile, recursive: bool) -> ExitCode {
    let mut violations = false;
    let Some(incomplete) = directory::walk(root, recursive, |parent, directory| {
        violations |= check_directory(parent, directory, profile);
    }) else {
        return ExitCode::from(2);
    };
    if incomplete {
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

fn check_directory(parent: &Path, directory: directory::Directory, profile: Profile) -> bool {
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
                    display(parent.join(name).as_os_str())
                );
                violations = true;
            }
        }
    }

    violations
}
