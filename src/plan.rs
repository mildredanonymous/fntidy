use std::{
    cmp::Ordering,
    collections::HashMap,
    ffi::OsStr,
    fmt,
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
    kinds: Vec<EntryKind>,
    collisions: Vec<Vec<String>>,
    blocked: EntryCounts,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    File,
    Folder,
}

impl EntryKind {
    fn of(entry: &directory::Entry) -> Self {
        if entry.is_dir {
            Self::Folder
        } else {
            Self::File
        }
    }
}

#[derive(Default)]
struct EntryCounts {
    files: usize,
    folders: usize,
}

impl EntryCounts {
    fn count(&mut self, kind: EntryKind) {
        match kind {
            EntryKind::File => self.files += 1,
            EntryKind::Folder => self.folders += 1,
        }
    }

    fn of(kinds: &[EntryKind]) -> Self {
        let mut counts = Self::default();
        for &kind in kinds {
            counts.count(kind);
        }
        counts
    }
}

impl fmt::Display for EntryCounts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} file{}, {} folder{}",
            self.files,
            if self.files == 1 { "" } else { "s" },
            self.folders,
            if self.folders == 1 { "" } else { "s" }
        )
    }
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
        kinds: Vec::new(),
        collisions: Vec::new(),
        blocked: EntryCounts::default(),
    };
    let incomplete = directory::walk(root, recursive, |parent, directory| {
        let local = preview_directory(parent, directory, profile);
        if local.status == Status::Blocked {
            preview.status = Status::Blocked;
        }
        preview.proposals.extend(local.proposals);
        preview.kinds.extend(local.kinds);
        preview.collisions.extend(local.collisions);
        preview.blocked.files += local.blocked.files;
        preview.blocked.folders += local.blocked.folders;
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
    let mut kinds = Vec::new();
    let mut collisions: Vec<Vec<String>> = Vec::new();
    let mut blocked = EntryCounts::default();
    for group in final_names
        .chunk_by(|left, right| windows_names::compare(&left.0, &right.0) == Ordering::Equal)
    {
        if group.len() > 1 {
            collisions.push(
                group
                    .iter()
                    .map(|(_, entry, _)| {
                        blocked.count(EntryKind::of(entry));
                        relative_name(parent, &entry.name)
                    })
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
            let kind = EntryKind::of(entry);
            if group.len() == 1 && status != ProposalStatus::Proposed {
                blocked.count(kind);
            }
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
            kinds.push(kind);
        }
    }

    let status = if blocked.files + blocked.folders > 0 {
        Status::Blocked
    } else {
        Status::Ready
    };

    Preview {
        status,
        proposals,
        kinds,
        collisions,
        blocked,
    }
}

pub fn run(root: &Path, output: Option<&Path>, profile: Profile, recursive: bool) -> ExitCode {
    let Some(preview) = preview(root, profile, recursive) else {
        return ExitCode::from(2);
    };
    print_preview(profile, &preview);
    let status = preview.status;

    if let Some(output) = output {
        if let Err(error) = export(root, output, profile, recursive, preview) {
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
            println!("No entries were renamed.");
            ExitCode::SUCCESS
        }
    }
}

fn print_preview(profile: Profile, preview: &Preview) {
    if !preview.proposals.is_empty() || !preview.collisions.is_empty() {
        println!("Rename preview (profile: {})", profile.name());
        print_proposals(preview);
    }
    println!(
        "{} proposed rename(s): {}.",
        preview.proposals.len(),
        EntryCounts::of(&preview.kinds)
    );
    println!("Blocked entries: {}.", preview.blocked);
}

fn print_proposals(preview: &Preview) {
    for collision in &preview.collisions {
        let names = collision
            .iter()
            .map(|name| format!("\"{}\"", preview_path(name)))
            .collect::<Vec<_>>()
            .join(", ");
        println!("\nCollision: These entries would share a Windows name: {names}");
    }

    let folder_targets: HashMap<_, _> = preview
        .proposals
        .iter()
        .zip(&preview.kinds)
        .filter(|(_, kind)| **kind == EntryKind::Folder && preview.status == Status::Ready)
        .map(|(proposal, _)| (proposal.source.as_str(), leaf_name(&proposal.target)))
        .collect();
    let mut previous_parent = None;
    for proposal in &preview.proposals {
        let (parent, source) = proposal
            .source
            .rsplit_once(MAIN_SEPARATOR)
            .unwrap_or((".", &proposal.source));
        let target = leaf_name(&proposal.target);
        if previous_parent != Some(parent) {
            println!("\nFolder: {}", preview_path(parent));
            previous_parent = Some(parent);
        }
        println!("\n  Original: \"{}\"", preview_name(source));
        println!("  Proposed: \"{}\"", preview_name(target));
        if let Some(path) = final_path(&proposal.target, &folder_targets) {
            println!("  Final path: \"{}\"", preview_path(&path));
        }
        if let Some(reason) = proposal.status.block_reason() {
            println!("  Blocked:  {reason}");
        }
    }
    println!();
}

fn leaf_name(path: &str) -> &str {
    path.rsplit_once(MAIN_SEPARATOR)
        .map_or(path, |(_, name)| name)
}

fn final_path(target: &str, folder_targets: &HashMap<&str, &str>) -> Option<String> {
    if folder_targets.is_empty() {
        return None;
    }
    let (parent, name) = target.rsplit_once(MAIN_SEPARATOR)?;
    let mut final_path = String::with_capacity(target.len());
    let mut start = 0;
    let mut changed = false;
    for end in parent
        .match_indices(MAIN_SEPARATOR)
        .map(|(index, _)| index)
        .chain(std::iter::once(parent.len()))
    {
        let component = &parent[start..end];
        let final_component = folder_targets
            .get(&parent[..end])
            .copied()
            .unwrap_or(component);
        changed |= component != final_component;
        final_path.push_str(final_component);
        final_path.push(MAIN_SEPARATOR);
        start = end + MAIN_SEPARATOR.len_utf8();
    }
    final_path.push_str(name);
    changed.then_some(final_path)
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
        Ok((report, kinds)) => {
            println!(
                "Verified {} proposed rename(s): {}. No entries were renamed.",
                report.proposals.len(),
                EntryCounts::of(&kinds),
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Cannot verify plan {}: {error}", display(path.as_os_str()));
            ExitCode::from(2)
        }
    }
}

fn verify_report(path: &Path) -> Result<(Report, Vec<EntryKind>), Box<dyn std::error::Error>> {
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
        Status::Blocked => return Err(format!("The current directory has a naming conflict or an invalid target. Blocked entries: {}. Run plan again to see the details.", current.blocked).into()),
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
    Ok((report, current.kinds))
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
    let (report, kinds) = match verify_report(path) {
        Ok(verified) => verified,
        Err(error) => {
            eprintln!("Cannot apply plan {}: {error}", display(path.as_os_str()));
            eprintln!("Changed: 0 files, 0 folders.");
            return ExitCode::from(2);
        }
    };

    if report.proposals.is_empty() {
        println!("The plan contains no renames. Changed: 0 files, 0 folders.");
        return ExitCode::SUCCESS;
    }
    let Some(identities) = &report.identities else {
        return ExitCode::from(2);
    };
    let root = Path::new(&report.root);
    let mut journal = match start_journal(&report.root, identities.root.clone()) {
        Ok(journal) => journal,
        Err(error) => {
            eprintln!("Cannot create the recovery journal: {error}");
            eprintln!("Changed: 0 files, 0 folders.");
            return ExitCode::from(2);
        }
    };
    if let Err(error) = execute_batch(
        &report.proposals,
        &kinds,
        identities,
        &mut journal,
        |proposal, parent_id, source_id| {
            crate::windows_rename::rename_from_root(
                root,
                &identities.root,
                parent_id,
                &proposal.source,
                source_id,
                &proposal.target,
            )
        },
    ) {
        eprintln!("{error} Keep the recovery journal.");
        return ExitCode::from(3);
    }

    println!(
        "{} rename(s) completed. Changed: {}.",
        report.proposals.len(),
        EntryCounts::of(&kinds)
    );
    ExitCode::SUCCESS
}

#[cfg(windows)]
fn execute_batch(
    proposals: &[Proposal],
    kinds: &[EntryKind],
    identities: &PlanIdentities,
    journal: &mut crate::journal::Journal,
    mut rename: impl FnMut(&Proposal, &Identity, &Identity) -> io::Result<()>,
) -> io::Result<()> {
    // A directory keeps its original path until all proposed edits below it are complete.
    let mut order: Vec<_> = (0..proposals.len()).collect();
    order.sort_by_cached_key(|&index| {
        std::cmp::Reverse(proposals[index].source.split(MAIN_SEPARATOR).count())
    });
    let mut changed = EntryCounts::default();
    for (completed, index) in order.into_iter().enumerate() {
        let proposal = &proposals[index];
        let source_id = &identities.sources[index];
        let parent_id = identities
            .parents
            .as_ref()
            .map_or(&identities.root, |parents| &parents[index]);
        journal
            .begin(&proposal.source, &proposal.target, source_id.clone(), Some(parent_id.clone()))
            .and_then(|()| rename(proposal, parent_id, source_id))
            .and_then(|()| journal.confirm())
            .map_err(|error| io::Error::other(format!(
                "Stopped after {completed} of {} confirmed rename(s). Changed (confirmed): {changed}. Could not complete \"{}\" -> \"{}\": {error}.",
                proposals.len(), preview_path(&proposal.source), preview_path(&proposal.target),
            )))?;
        changed.count(kinds[index]);
        println!(
            "Renamed \"{}\" to \"{}\".",
            preview_path(&proposal.source),
            preview_path(&proposal.target)
        );
    }
    journal.finish().map_err(|error| {
        io::Error::other(format!(
            "All {} rename(s) were confirmed. Changed: {changed}. Could not finalize the journal: {error}.",
            proposals.len(),
        ))
    })
}

#[cfg(all(test, windows))]
mod batch_tests {
    use super::*;
    use std::fs;

    #[test]
    fn stops_a_batch_when_a_target_becomes_occupied_during_apply() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().join("test-tree");
        fs::create_dir(&root).unwrap();
        for name in ["A 😀.txt", "B 😀.txt", "C 😀.txt"] {
            fs::write(root.join(name), name).unwrap();
        }
        let plan_path = workspace.path().join("plan.json");
        let profile = Profile::Tidy;
        export(
            &root,
            &plan_path,
            profile,
            true,
            preview(&root, profile, true).unwrap(),
        )
        .unwrap();
        let (report, kinds) = verify_report(&plan_path).unwrap();
        let identities = report.identities.unwrap();
        let log = workspace.path().join("run.jsonl");
        let mut journal =
            crate::journal::Journal::create(&log, &report.root, identities.root.clone()).unwrap();
        let result = execute_batch(
            &report.proposals,
            &kinds,
            &identities,
            &mut journal,
            |proposal, parent_id, source_id| {
                if proposal.source == "B 😀.txt" {
                    fs::write(root.join("B.txt"), "external file")?;
                }
                crate::windows_rename::rename_from_root(
                    &root,
                    &identities.root,
                    parent_id,
                    &proposal.source,
                    source_id,
                    &proposal.target,
                )
            },
        );
        let message = result.unwrap_err().to_string();
        assert!(message.contains("Stopped after 1 of 3 confirmed"));
        assert!(message.contains("Changed (confirmed): 1 file, 0 folders."));
        drop(journal);

        assert_eq!(fs::read_to_string(root.join("A.txt")).unwrap(), "A 😀.txt");
        assert!(!root.join("A 😀.txt").exists());
        assert_eq!(
            fs::read_to_string(root.join("B.txt")).unwrap(),
            "external file"
        );
        assert_eq!(
            fs::read_to_string(root.join("B 😀.txt")).unwrap(),
            "B 😀.txt"
        );
        assert_eq!(
            fs::read_to_string(root.join("C 😀.txt")).unwrap(),
            "C 😀.txt"
        );
        assert!(!root.join("C.txt").exists());
        let records: Vec<serde_json::Value> = fs::read_to_string(log)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 4);
        assert_eq!(records[1]["source"], "A 😀.txt");
        assert_eq!(records[2]["event"], "renamed");
        assert_eq!(records[3]["event"], "pending");
        assert_eq!(records[3]["source"], "B 😀.txt");
    }
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
    match path.to_str() {
        Some(path) if !path.chars().any(char::is_control) => eprintln!("Recovery journal: {path}"),
        _ => eprintln!("Recovery journal: {}", display(path.as_os_str())),
    }
    Ok(journal)
}
