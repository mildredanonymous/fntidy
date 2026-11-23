use std::{cmp::Ordering, ffi::OsStr, path::Path, process::ExitCode};

use crate::{
    directory::{self, display},
    windows_names,
};

pub fn run(root: &Path) -> ExitCode {
    let Some(directory) = directory::scan(root) else {
        return ExitCode::from(2);
    };
    let mut final_names = directory
        .entries
        .iter()
        .map(|entry| {
            let proposal = if entry.redirected {
                None
            } else {
                windows_names::repair(&entry.name)
            };
            let key = proposal
                .as_deref()
                .map_or_else(|| entry.key.clone(), windows_names::comparison_units);
            (key, entry, proposal)
        })
        .collect::<Vec<_>>();
    final_names.sort_unstable_by(|left, right| {
        windows_names::compare(&left.0, &right.0).then_with(|| left.1.name.cmp(&right.1.name))
    });

    let mut proposals = 0;
    let mut blocked = false;
    for group in final_names
        .chunk_by(|left, right| windows_names::compare(&left.0, &right.0) == Ordering::Equal)
    {
        if group.len() > 1 {
            blocked = true;
            let names = group
                .iter()
                .map(|(_, entry, _)| display(OsStr::new(&entry.name)))
                .collect::<Vec<_>>()
                .join(", ");
            println!(
                "[windows] WIN_NAME_COLLISION: These entries would share a Windows name: {names}"
            );
        }
        for (key, entry, proposal) in group {
            let Some(target) = proposal else { continue };
            proposals += 1;
            let occupied = directory.entries.as_slice().partition_point(|sibling| {
                windows_names::compare(&sibling.key, key) == Ordering::Less
            });
            let occupied_by_sibling = directory.entries[occupied..]
                .iter()
                .take_while(|sibling| windows_names::compare(&sibling.key, key) == Ordering::Equal)
                .any(|sibling| sibling.name != entry.name);
            let invalid = windows_names::violations(target).next().is_some()
                || target.encode_utf16().count() > 255;
            let status = if invalid {
                "blocked: target needs manual review"
            } else if group.len() > 1 {
                "blocked: proposed names collide"
            } else if occupied_by_sibling {
                "blocked: target matches an existing sibling"
            } else {
                "proposed"
            };
            blocked |= invalid || occupied_by_sibling;
            let rules = windows_names::violations(&entry.name)
                .map(|rule| rule.diagnostic().0)
                .collect::<Vec<_>>()
                .join(", ");
            println!(
                "[windows] {} -> {} ({status}; {rules})",
                display(OsStr::new(&entry.name)),
                display(OsStr::new(target)),
            );
        }
    }

    if directory.incomplete {
        eprintln!("The rename preview was incomplete.");
        ExitCode::from(2)
    } else if blocked {
        println!("Preview blocked by unresolved names. No entries were renamed.");
        ExitCode::from(1)
    } else {
        println!("{proposals} proposed rename(s). No entries were renamed.");
        ExitCode::SUCCESS
    }
}
