//! Append-only records for a run. Only newline-terminated records are committed.

use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::Path,
};

use serde::{Deserialize, Serialize};

use crate::identity::Identity;

const VERSION: u32 = 1;

#[derive(Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
enum Record {
    Run {
        version: u32,
        root: String,
        root_identity: Identity,
    },
    Pending {
        source: String,
        target: String,
        source_identity: Identity,
    },
    Renamed,
    Finished,
}

pub struct Journal {
    file: File,
    state: State,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ready,
    Pending,
    Finished,
    Failed,
}

impl Journal {
    pub fn create(path: &Path, root: &str, root_identity: Identity) -> io::Result<Self> {
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        let mut journal = Self {
            file,
            state: State::Ready,
        };
        journal.append(
            &Record::Run {
                version: VERSION,
                root: root.to_owned(),
                root_identity,
            },
            State::Ready,
        )?;
        Ok(journal)
    }

    pub fn begin(
        &mut self,
        source: &str,
        target: &str,
        source_identity: Identity,
    ) -> io::Result<()> {
        self.require(State::Ready)?;
        self.append(
            &Record::Pending {
                source: source.to_owned(),
                target: target.to_owned(),
                source_identity,
            },
            State::Pending,
        )
    }

    pub fn confirm(&mut self) -> io::Result<()> {
        self.require(State::Pending)?;
        self.append(&Record::Renamed, State::Ready)
    }

    pub fn finish(&mut self) -> io::Result<()> {
        self.require(State::Ready)?;
        self.append(&Record::Finished, State::Finished)
    }

    fn require(&self, expected: State) -> io::Result<()> {
        if self.state == expected {
            Ok(())
        } else {
            Err(io::Error::other("journal event is out of sequence"))
        }
    }

    fn append(&mut self, record: &Record, next: State) -> io::Result<()> {
        self.state = State::Failed;
        let mut line = serde_json::to_vec(record)?;
        line.push(b'\n');
        self.file.write_all(&line)?;
        self.file.sync_all()?;
        self.state = next;
        Ok(())
    }
}

#[cfg(test)]
use std::io::{BufRead, BufReader};

#[cfg(test)]
#[derive(Debug, PartialEq)]
struct Step {
    source: String,
    target: String,
    source_identity: Identity,
}

#[cfg(test)]
#[derive(Debug)]
struct Run {
    root: String,
    root_identity: Identity,
    confirmed: Vec<Step>,
    pending: Option<Step>,
    finished: bool,
}

#[cfg(test)]
fn inspect(path: &Path) -> io::Result<Run> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut line = Vec::new();
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid or unsupported run journal",
        )
    };
    let Some(Record::Run {
        version: VERSION,
        root,
        root_identity,
    }) = next_record(&mut reader, &mut line)?
    else {
        return Err(invalid());
    };
    let mut run = Run {
        root,
        root_identity,
        confirmed: Vec::new(),
        pending: None,
        finished: false,
    };
    while let Some(record) = next_record(&mut reader, &mut line)? {
        match record {
            Record::Pending {
                source,
                target,
                source_identity,
            } if !run.finished && run.pending.is_none() => {
                run.pending = Some(Step {
                    source,
                    target,
                    source_identity,
                });
            }
            Record::Renamed => run.confirmed.push(run.pending.take().ok_or_else(invalid)?),
            Record::Finished if !run.finished && run.pending.is_none() => run.finished = true,
            _ => return Err(invalid()),
        }
    }
    Ok(run)
}

#[cfg(test)]
fn next_record(reader: &mut impl BufRead, line: &mut Vec<u8>) -> io::Result<Option<Record>> {
    line.clear();
    reader.read_until(b'\n', line)?;
    if !line.ends_with(b"\n") {
        return Ok(None);
    }
    serde_json::from_slice(line)
        .map(Some)
        .map_err(io::Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_pending_confirmed_and_finished_steps() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path();
        let log = root.join("run.jsonl");
        let root_name = root.to_str().unwrap();
        let root_id = || crate::identity::read(root).unwrap();
        let source = root.join("before");
        std::fs::write(&source, b"original").unwrap();
        let source_id = || crate::identity::read(&source).unwrap();

        let mut journal = Journal::create(&log, root_name, root_id()).unwrap();
        assert!(Journal::create(&log, root_name, root_id()).is_err());
        assert!(journal.confirm().is_err());
        journal.begin("before", "after", source_id()).unwrap();
        assert!(journal.finish().is_err());
        let run = inspect(&log).unwrap();
        assert_eq!(run.root, root_name);
        assert_eq!(run.root_identity, root_id());
        assert_eq!(run.pending.unwrap().source_identity, source_id());
        assert!(run.confirmed.is_empty());

        journal.confirm().unwrap();
        journal.finish().unwrap();
        assert!(journal.begin("before", "after", source_id()).is_err());
        let run = inspect(&log).unwrap();
        assert_eq!(run.confirmed.len(), 1);
        assert!(run.pending.is_none());
        assert!(run.finished);
    }

    #[test]
    fn leaves_a_step_pending_when_confirmation_is_torn() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path();
        let log = root.join("run.jsonl");
        let root_id = || crate::identity::read(root).unwrap();
        let mut journal = Journal::create(&log, root.to_str().unwrap(), root_id()).unwrap();
        journal.begin("before", "after", root_id()).unwrap();
        journal.file.write_all(br#"{"event":"renam"#).unwrap();
        journal.file.sync_all().unwrap();
        drop(journal);

        let run = inspect(&log).unwrap();
        assert_eq!(run.pending.unwrap().target, "after");
        assert!(!run.finished);
    }

    #[cfg(windows)]
    #[test]
    fn detects_interruption_after_a_physical_rename() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path();
        let log = root.join("run.jsonl");
        let source = root.join("before");
        let target = root.join("after");
        std::fs::write(&source, b"original").unwrap();
        let root_id = crate::identity::read(root).unwrap();
        let source_id = crate::identity::read(&source).unwrap();
        let mut journal = Journal::create(
            &log,
            root.to_str().unwrap(),
            crate::identity::read(root).unwrap(),
        )
        .unwrap();

        journal
            .begin("before", "after", crate::identity::read(&source).unwrap())
            .unwrap();
        crate::windows_rename::rename(root, &root_id, "before", &source_id, "after").unwrap();
        drop(journal);

        let run = inspect(&log).unwrap();
        let step = run.pending.unwrap();
        assert_eq!(
            (step.source.as_str(), step.target.as_str()),
            ("before", "after")
        );
        assert_eq!(
            crate::identity::read(&target).unwrap(),
            step.source_identity
        );
        assert!(!source.exists());
        assert!(!run.finished);
    }
}
