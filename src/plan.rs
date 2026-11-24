use std::{
    cmp::Ordering,
    ffi::OsStr,
    fs::File,
    io::BufReader,
    io::{self, Write},
    path::{MAIN_SEPARATOR, Path},
    process::ExitCode,
};

use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    directory::{self, display},
    identity::{self, Identity},
    profile::Profile,
    windows_names,
};

const SCHEMA_VERSION: u32 = 2;
const RECURSIVE_SCHEMA_VERSION: u32 = 3;

fn schema_version(recursive: bool) -> u32 {
    if recursive {
        RECURSIVE_SCHEMA_VERSION
    } else {
        SCHEMA_VERSION
    }
}

fn is_flat(recursive: &bool) -> bool {
    !recursive
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Report {
    schema_version: u32,
    root: String,
    profile: String,
    profile_version: u32,
    #[serde(default, skip_serializing_if = "is_flat")]
    recursive: bool,
    status: Status,
    proposals: Vec<Proposal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    identities: Option<PlanIdentities>,
    collisions: Vec<Vec<String>>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PlanIdentities {
    root: Identity,
    sources: Vec<Identity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parents: Option<Vec<Identity>>,
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
    fn block_reason(self) -> Option<&'static str> {
        match self {
            Self::Proposed => None,
            Self::InvalidTarget => Some("Target needs manual review."),
            Self::ProposedCollision => Some("Proposed names collide."),
            Self::ExistingSibling => Some("Target matches an existing sibling."),
        }
    }
}

fn relative_name(parent: &Path, name: &str) -> String {
    if parent.as_os_str().is_empty() {
        name.to_owned()
    } else {
        format!("{}{MAIN_SEPARATOR}{name}", parent.display())
    }
}

fn preview(root: &Path, profile: Profile, recursive: bool) -> Option<Preview> {
    let mut preview = Preview {
        status: Status::Ready,
        proposals: Vec::new(),
        collisions: Vec::new(),
    };
    let incomplete = directory::walk(root, recursive, |parent, directory| {
        let local = preview_directory(parent, directory, profile);
        if local.status == Status::Blocked {
            preview.status = Status::Blocked;
        }
        preview.proposals.extend(local.proposals);
        preview.collisions.extend(local.collisions);
    })?;
    if incomplete {
        preview.status = Status::Incomplete;
    }
    Some(preview)
}

fn preview_directory(parent: &Path, directory: directory::Directory, profile: Profile) -> Preview {
    let mut final_names = directory
        .entries
        .iter()
        .map(|entry| {
            let proposal = if entry.redirected {
                None
            } else {
                profile.repair(&entry.name)
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
                    .map(|(_, entry, _)| relative_name(parent, &entry.name))
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
            let invalid =
                profile.violations(target).next().is_some() || target.encode_utf16().count() > 255;
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
            let rules = profile
                .violations(&entry.name)
                .map(|(id, _)| id.to_owned())
                .collect();
            proposals.push(Proposal {
                source: relative_name(parent, &entry.name),
                target: relative_name(parent, target),
                rules,
                status,
            });
        }
    }

    let status = if blocked {
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

pub fn run(root: &Path, output: Option<&Path>, profile: Profile, recursive: bool) -> ExitCode {
    let Some(Preview {
        status,
        proposals,
        collisions,
    }) = preview(root, profile, recursive)
    else {
        return ExitCode::from(2);
    };
    let proposal_count = proposals.len();

    print_preview(profile, &proposals, &collisions);

    if let Some(output) = output {
        if let Err(error) = export(
            root,
            output,
            profile,
            recursive,
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

fn print_preview(profile: Profile, proposals: &[Proposal], collisions: &[Vec<String>]) {
    if proposals.is_empty() && collisions.is_empty() {
        return;
    }
    println!("Rename preview (profile: {})", profile.name());

    for collision in collisions {
        let names = collision
            .iter()
            .map(|name| format!("\"{}\"", preview_path(name)))
            .collect::<Vec<_>>()
            .join(", ");
        println!("\nCollision: These entries would share a Windows name: {names}");
    }

    let mut previous_parent = None;
    for proposal in proposals {
        let (parent, source) = proposal
            .source
            .rsplit_once(MAIN_SEPARATOR)
            .unwrap_or((".", &proposal.source));
        let target = proposal
            .target
            .rsplit(MAIN_SEPARATOR)
            .next()
            .expect("a target name");
        if previous_parent != Some(parent) {
            println!("\nFolder: {}", preview_path(parent));
            previous_parent = Some(parent);
        }
        println!("\n  Original: \"{}\"", preview_name(source));
        println!("  Proposed: \"{}\"", preview_name(target));
        if let Some(reason) = proposal.status.block_reason() {
            println!("  Blocked:  {reason}");
        }
    }
    println!();
}

fn preview_path(path: &str) -> String {
    path.split(MAIN_SEPARATOR)
        .map(preview_name)
        .collect::<Vec<_>>()
        .join(std::path::MAIN_SEPARATOR_STR)
}

fn preview_name(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    let bytes = name.as_bytes();
    for (index, grapheme) in name.grapheme_indices(true) {
        if grapheme == " "
            && (index == 0
                || bytes[index - 1] == b' '
                || matches!(bytes.get(index + 1), None | Some(b' ')))
        {
            result.push('␠');
        } else if grapheme == "␠" {
            result.push_str("\\u{2420}");
        } else if emojis::get(grapheme).is_some() || grapheme == "'" {
            result.push_str(grapheme);
        } else {
            result.extend(grapheme.escape_debug());
        }
    }
    result
}

#[cfg(test)]
mod display_tests {
    use super::preview_name;

    #[test]
    fn preview_displays_emoji_and_exposes_invisible_characters() {
        assert_eq!(preview_name("Familie 👨‍👩‍👧‍👦.txt"), "Familie 👨‍👩‍👧‍👦.txt");
        assert_eq!(preview_name("Herz ❤️.txt"), "Herz ❤️.txt");
        assert_eq!(preview_name("Café.txt"), "Café.txt");
        assert_eq!(preview_name("Cafe\u{301}.txt"), r"Cafe\u{301}.txt");
        assert_eq!(preview_name(" zwei  Namen.txt "), "␠zwei␠␠Namen.txt␠");
        assert_eq!(
            preview_name("␠\u{202f}\u{202e}\u{1b}\n"),
            r"\u{2420}\u{202f}\u{202e}\u{1b}\n"
        );
    }
}

pub fn verify(path: &Path) -> ExitCode {
    match verify_report(path) {
        Ok(report) => {
            println!(
                "Verified {} proposed rename(s). No entries were renamed.",
                report.proposals.len()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Cannot verify plan {}: {error}", display(path.as_os_str()));
            ExitCode::from(2)
        }
    }
}

fn verify_report(path: &Path) -> Result<Report, Box<dyn std::error::Error>> {
    let report: Report = serde_json::from_reader(BufReader::new(File::open(path)?))?;
    let unsupported = "Unsupported plan format or rules. Create a fresh plan.";
    let profile = Profile::parse(OsStr::new(&report.profile)).ok_or(unsupported)?;
    if report.schema_version != schema_version(report.recursive)
        || report.profile_version != profile.version()
    {
        return Err(unsupported.into());
    }
    if report.status != Status::Ready || !report.collisions.is_empty() {
        return Err("The saved plan is blocked or incomplete. Create a fresh plan.".into());
    }

    let root = Path::new(&report.root);
    if !root.is_absolute() {
        return Err("The saved plan must specify an absolute directory.".into());
    }
    let current =
        preview(root, profile, report.recursive).ok_or("Cannot scan the saved directory.")?;
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
    let expected = report
        .identities
        .as_ref()
        .ok_or("The saved plan lacks file identities. Create a fresh plan.")?;
    let actual = capture_identities(root, &current.proposals, report.recursive)?;
    if expected.root != actual.root {
        return Err("The saved directory was replaced. Create a fresh plan.".into());
    }
    if expected.parents != actual.parents {
        return Err("A parent directory was replaced or the plan lacks parent identities. Create a fresh plan.".into());
    }
    if expected.sources.len() != actual.sources.len() {
        return Err("The saved plan lacks source identities. Create a fresh plan.".into());
    }
    for ((expected, actual), proposal) in expected
        .sources
        .iter()
        .zip(&actual.sources)
        .zip(&current.proposals)
    {
        if expected != actual {
            return Err(format!(
                "The source {} was replaced. Create a fresh plan.",
                display(OsStr::new(&proposal.source))
            )
            .into());
        }
    }
    Ok(report)
}

fn capture_identities(
    root: &Path,
    proposals: &[Proposal],
    recursive: bool,
) -> io::Result<PlanIdentities> {
    let root_identity = identity::read(root)?;
    // On Windows canonicalize gives a verbatim base path for exact entry names.
    let base = root.canonicalize()?;
    if identity::read(&base)? != root_identity {
        return Err(io::Error::other(
            "the directory changed while recording identities",
        ));
    }
    let mut sources = Vec::with_capacity(proposals.len());
    let mut parents = recursive.then(|| Vec::with_capacity(proposals.len()));
    for proposal in proposals {
        let mut source_path = base.clone();
        let mut parent_id = root_identity.clone();
        let mut components = proposal.source.split(MAIN_SEPARATOR).peekable();
        while let Some(component) = components.next() {
            if !directory::is_name_component(component)
                || (!recursive && components.peek().is_some())
            {
                return Err(io::Error::other(
                    "the source has an ambiguous path component",
                ));
            }
            source_path.push(component);
            let entry_id = identity::read(&source_path).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!(
                        "Cannot read the identity of {}: {error}",
                        display(source_path.as_os_str())
                    ),
                )
            })?;
            if components.peek().is_some() {
                parent_id = entry_id;
            } else {
                sources.push(entry_id);
            }
        }
        if let Some(parents) = &mut parents {
            parents.push(parent_id);
        }
    }
    Ok(PlanIdentities {
        root: root_identity,
        sources,
        parents,
    })
}

fn export(
    root: &Path,
    output: &Path,
    profile: Profile,
    recursive: bool,
    preview: Preview,
) -> io::Result<()> {
    let absolute_root = std::path::absolute(root)?;
    let root = absolute_root.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "the directory path cannot be represented as Unicode",
        )
    })?;
    let report = Report {
        schema_version: schema_version(recursive),
        root: root.to_owned(),
        profile: profile.name().to_owned(),
        profile_version: profile.version(),
        recursive,
        status: preview.status,
        identities: (preview.status == Status::Ready)
            .then(|| capture_identities(&absolute_root, &preview.proposals, recursive))
            .transpose()?,
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

#[cfg(windows)]
pub fn apply(path: &Path) -> ExitCode {
    let report = match verify_report(path) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("Cannot apply plan {}: {error}", display(path.as_os_str()));
            return ExitCode::from(2);
        }
    };

    let [proposal] = report.proposals.as_slice() else {
        if report.proposals.is_empty() {
            println!("The plan contains no renames.");
            return ExitCode::SUCCESS;
        }
        eprintln!("This preview applies only plans with one proposed file rename.");
        return ExitCode::from(2);
    };
    let Some(PlanIdentities {
        root: root_id,
        sources,
        parents,
    }) = report.identities
    else {
        return ExitCode::from(2);
    };
    let [source_id] = sources.as_slice() else {
        return ExitCode::from(2);
    };
    let parent_id = parents
        .as_deref()
        .and_then(|parents| parents.first())
        .unwrap_or(&root_id);
    let root = Path::new(&report.root);
    let mut journal = match start_journal(&report.root, root_id.clone()) {
        Ok(journal) => journal,
        Err(error) => {
            eprintln!("Cannot create the recovery journal: {error}");
            return ExitCode::from(2);
        }
    };
    if let Err(error) = journal.begin(&proposal.source, &proposal.target, source_id.clone()) {
        eprintln!("Cannot record the rename: {error}");
        return ExitCode::from(2);
    }

    let result = crate::windows_rename::rename_from_root(
        root,
        &root_id,
        parent_id,
        &proposal.source,
        source_id,
        &proposal.target,
    )
    .and_then(|()| journal.confirm())
    .and_then(|()| journal.finish());
    if let Err(error) = result {
        eprintln!(
            "The run needs review. Keep the journal and check the source and target: {error}"
        );
        return ExitCode::from(3);
    }

    println!(
        "Renamed {} to {}.",
        display(OsStr::new(&proposal.source)),
        display(OsStr::new(&proposal.target)),
    );
    ExitCode::SUCCESS
}

#[cfg(windows)]
fn start_journal(root: &str, root_id: Identity) -> io::Result<crate::journal::Journal> {
    let local = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| io::Error::other("LOCALAPPDATA must be an absolute path"))?;
    let logs = local.join("fntidy").join("runs");
    std::fs::create_dir_all(&logs)?;
    let directory = tempfile::Builder::new().prefix("run-").tempdir_in(logs)?;
    let path = directory.path().join("run.jsonl");
    let journal = crate::journal::Journal::create(&path, root, root_id)?;
    let _ = directory.keep();
    eprintln!("Recovery journal: {}", display(path.as_os_str()));
    Ok(journal)
}
