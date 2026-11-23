use std::{
    cmp::Ordering,
    ffi::OsStr,
    fs::File,
    io::BufReader,
    io::{self, Write},
    path::Path,
    process::ExitCode,
};

use serde::{Deserialize, Serialize};

use crate::{
    directory::{self, display},
    windows_names,
};

const SCHEMA_VERSION: u32 = 1;
const PROFILE_VERSION: u32 = 1;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Report {
    schema_version: u32,
    root: String,
    profile: String,
    profile_version: u32,
    status: Status,
    proposals: Vec<Proposal>,
    collisions: Vec<Vec<String>>,
}

struct Preview {
    status: Status,
    proposals: Vec<Proposal>,
    collisions: Vec<Vec<String>>,
}

#[derive(Clone, Copy, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Ready,
    Blocked,
    Incomplete,
}

#[derive(Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    source: String,
    target: String,
    rules: Vec<String>,
    status: ProposalStatus,
}

#[derive(Clone, Copy, Deserialize, PartialEq, Serialize)]
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

fn preview(directory: directory::Directory) -> Preview {
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
    let mut collisions: Vec<Vec<String>> = Vec::new();
    let mut blocked = false;
    for group in final_names
        .chunk_by(|left, right| windows_names::compare(&left.0, &right.0) == Ordering::Equal)
    {
        if group.len() > 1 {
            blocked = true;
            collisions.push(
                group
                    .iter()
                    .map(|(_, entry, _)| entry.name.clone())
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
                .map(|rule| rule.diagnostic().0.to_owned())
                .collect();
            proposals.push(Proposal {
                source: entry.name.clone(),
                target: target.clone(),
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

    Preview {
        status,
        proposals,
        collisions,
    }
}

pub fn run(root: &Path, output: Option<&Path>) -> ExitCode {
    let Some(directory) = directory::scan(root) else {
        return ExitCode::from(2);
    };
    let Preview {
        status,
        proposals,
        collisions,
    } = preview(directory);
    let proposal_count = proposals.len();

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
            display(OsStr::new(&proposal.source)),
            display(OsStr::new(&proposal.target)),
            proposal.status.description(),
            proposal.rules.join(", "),
        );
    }

    if let Some(output) = output {
        if let Err(error) = export(
            root,
            output,
            Preview {
                status,
                proposals,
                collisions,
            },
        ) {
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
                proposal_count
            );
            ExitCode::SUCCESS
        }
    }
}

pub fn verify(path: &Path) -> ExitCode {
    match verify_report(path) {
        Ok(count) => {
            println!("Verified {count} proposed rename(s). No entries were renamed.");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Cannot verify plan {}: {error}", display(path.as_os_str()));
            ExitCode::from(2)
        }
    }
}

fn verify_report(path: &Path) -> Result<usize, Box<dyn std::error::Error>> {
    let report: Report = serde_json::from_reader(BufReader::new(File::open(path)?))?;
    if report.schema_version != SCHEMA_VERSION
        || report.profile != windows_names::PROFILE
        || report.profile_version != PROFILE_VERSION
    {
        return Err("Unsupported plan format or rules. Create a fresh plan.".into());
    }
    if report.status != Status::Ready || !report.collisions.is_empty() {
        return Err("The saved plan is blocked or incomplete. Create a fresh plan.".into());
    }

    let root = Path::new(&report.root);
    if !root.is_absolute() {
        return Err("The saved plan must specify an absolute directory.".into());
    }
    let directory = directory::scan(root).ok_or("Cannot scan the saved directory.")?;
    let current = preview(directory);
    match current.status {
        Status::Incomplete => return Err("The current scan is incomplete. Create a fresh plan after resolving the scan errors.".into()),
        Status::Blocked => return Err("The current directory has a naming conflict or an invalid target. Run plan again to see the details.".into()),
        Status::Ready => {}
    }
    if report.proposals != current.proposals {
        return Err(
            "The saved proposals differ from the current preview. Create a fresh plan.".into(),
        );
    }
    Ok(report.proposals.len())
}

fn export(root: &Path, output: &Path, preview: Preview) -> io::Result<()> {
    let absolute_root = std::path::absolute(root)?;
    let root = absolute_root.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "the directory path cannot be represented as Unicode",
        )
    })?;
    let report = Report {
        schema_version: SCHEMA_VERSION,
        root: root.to_owned(),
        profile: windows_names::PROFILE.to_owned(),
        profile_version: PROFILE_VERSION,
        status: preview.status,
        proposals: preview.proposals,
        collisions: preview.collisions,
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
