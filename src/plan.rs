use std::{
    cmp::Ordering,
    ffi::OsStr,
    io::{self, Write},
    path::Path,
    process::ExitCode,
};

use serde::Serialize;

use crate::{
    directory::{self, display},
    windows_names,
};

#[derive(Serialize)]
struct Report<'a> {
    schema_version: u32,
    root: &'a str,
    profile: &'static str,
    profile_version: u32,
    status: Status,
    proposals: &'a [Proposal<'a>],
    collisions: Vec<Vec<&'a str>>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Ready,
    Blocked,
    Incomplete,
}

#[derive(Serialize)]
struct Proposal<'a> {
    source: &'a str,
    target: &'a str,
    rules: Vec<&'static str>,
    status: ProposalStatus,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProposalStatus {
    Proposed,
    InvalidTarget,
    ProposedCollision,
    ExistingSibling,
}

impl ProposalStatus {
    fn description(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::InvalidTarget => "blocked: target needs manual review",
            Self::ProposedCollision => "blocked: proposed names collide",
            Self::ExistingSibling => "blocked: target matches an existing sibling",
        }
    }
}

pub fn run(root: &Path, output: Option<&Path>) -> ExitCode {
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

    let mut proposals = Vec::new();
    let mut collisions: Vec<Vec<&str>> = Vec::new();
    let mut blocked = false;
    for group in final_names
        .chunk_by(|left, right| windows_names::compare(&left.0, &right.0) == Ordering::Equal)
    {
        if group.len() > 1 {
            blocked = true;
            collisions.push(
                group
                    .iter()
                    .map(|(_, entry, _)| entry.name.as_str())
                    .collect(),
            );
        }
        for (key, entry, proposal) in group {
            let Some(target) = proposal else { continue };
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
                ProposalStatus::InvalidTarget
            } else if group.len() > 1 {
                ProposalStatus::ProposedCollision
            } else if occupied_by_sibling {
                ProposalStatus::ExistingSibling
            } else {
                ProposalStatus::Proposed
            };
            blocked |= invalid || occupied_by_sibling;
            let rules = windows_names::violations(&entry.name)
                .map(|rule| rule.diagnostic().0)
                .collect();
            proposals.push(Proposal {
                source: &entry.name,
                target,
                rules,
                status,
            });
        }
    }

    let status = if directory.incomplete {
        Status::Incomplete
    } else if blocked {
        Status::Blocked
    } else {
        Status::Ready
    };

    for collision in &collisions {
        let names = collision
            .iter()
            .map(|name| display(OsStr::new(name)))
            .collect::<Vec<_>>()
            .join(", ");
        println!("[windows] WIN_NAME_COLLISION: These entries would share a Windows name: {names}");
    }
    for proposal in &proposals {
        println!(
            "[windows] {} -> {} ({}; {})",
            display(OsStr::new(proposal.source)),
            display(OsStr::new(proposal.target)),
            proposal.status.description(),
            proposal.rules.join(", "),
        );
    }

    if let Some(output) = output {
        if let Err(error) = export(root, output, status, proposals.as_slice(), collisions) {
            eprintln!("Cannot write plan {}: {error}", display(output.as_os_str()));
            return ExitCode::from(2);
        }
        println!("Saved plan to {}.", display(output.as_os_str()));
    }

    match status {
        Status::Incomplete => {
            eprintln!("The rename preview was incomplete.");
            ExitCode::from(2)
        }
        Status::Blocked => {
            println!("Preview blocked by unresolved names. No entries were renamed.");
            ExitCode::from(1)
        }
        Status::Ready => {
            println!(
                "{} proposed rename(s). No entries were renamed.",
                proposals.len()
            );
            ExitCode::SUCCESS
        }
    }
}

fn export<'a>(
    root: &Path,
    output: &Path,
    status: Status,
    proposals: &'a [Proposal<'a>],
    collisions: Vec<Vec<&'a str>>,
) -> io::Result<()> {
    let absolute_root = std::path::absolute(root)?;
    let root = absolute_root.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "the directory path cannot be represented as Unicode",
        )
    })?;
    let report = Report {
        schema_version: 1,
        root,
        profile: windows_names::PROFILE,
        profile_version: 1,
        status,
        proposals,
        collisions,
    };
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(file.as_file_mut(), &report)?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist_noclobber(output)?;
    Ok(())
}
