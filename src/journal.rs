//! Append-only records for a run. Only newline-terminated records are committed.

use std::{
    fs::{File, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    path::Path,
};

use serde::{Deserialize, Serialize};

use crate::identity::Identity;

#[cfg(windows)]
use std::io::{Seek, SeekFrom};

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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_identity: Option<Identity>,
    },
    Renamed,
    Finished,
    Abandoned,
    UndoStarted,
    UndoPending {
        step: usize,
    },
    Undone {
        step: usize,
    },
    UndoFinished,
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
        let file = journal_options().write(true).create_new(true).open(path)?;
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
        parent_identity: Option<Identity>,
    ) -> io::Result<()> {
        self.require(State::Ready)?;
        self.append(
            &Record::Pending {
                source: source.to_owned(),
                target: target.to_owned(),
                source_identity,
                parent_identity,
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
        append_record(&mut self.file, record)?;
        self.state = next;
        Ok(())
    }
}

fn journal_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
        };
        // A recovery reader also denies sharing with any active writer.
        options
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options.read(true);
    options
}

fn append_record(file: &mut File, record: &Record) -> io::Result<()> {
    let mut line = serde_json::to_vec(record)?;
    line.push(b'\n');
    file.write_all(&line)?;
    file.sync_all()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub source: String,
    pub target: String,
    pub source_identity: Identity,
    pub parent_identity: Option<Identity>,
}

#[derive(Debug)]
pub struct Run {
    pub root: String,
    pub root_identity: Identity,
    pub confirmed: Vec<Step>,
    pub pending: Option<Step>,
    pub finished: bool,
    pub undo_started: bool,
    pub undo_pending: bool,
    pub undone: usize,
    pub undo_finished: bool,
    valid_bytes: u64,
    pub torn_tail: bool,
}

impl Run {
    #[cfg(windows)]
    pub fn next_undo(&self) -> Option<(usize, &Step)> {
        let number = self.confirmed.len() - self.undone;
        self.confirmed[..number].last().map(|step| (number, step))
    }

    fn record(&mut self, record: Record) -> io::Result<()> {
        let remaining = self.confirmed.len() - self.undone;
        match record {
            Record::Pending {
                source,
                target,
                source_identity,
                parent_identity,
            } if !self.finished && !self.undo_started && self.pending.is_none() => {
                self.pending = Some(Step {
                    source,
                    target,
                    source_identity,
                    parent_identity,
                });
            }
            Record::Renamed if !self.undo_started => {
                self.confirmed
                    .push(self.pending.take().ok_or_else(invalid)?);
            }
            Record::Abandoned if !self.undo_started && self.pending.is_some() => {
                self.pending = None
            }
            Record::Finished if !self.finished && !self.undo_started && self.pending.is_none() => {
                self.finished = true
            }
            Record::UndoStarted if !self.undo_started && self.pending.is_none() => {
                self.undo_started = true
            }
            Record::UndoPending { step }
                if self.undo_started
                    && !self.undo_finished
                    && !self.undo_pending
                    && step == remaining
                    && step > 0 =>
            {
                self.undo_pending = true;
            }
            Record::Undone { step } if self.undo_pending && step == remaining => {
                self.undone += 1;
                self.undo_pending = false;
            }
            Record::UndoFinished
                if self.undo_started
                    && !self.undo_finished
                    && !self.undo_pending
                    && remaining == 0 =>
            {
                self.undo_finished = true;
            }
            _ => return Err(invalid()),
        }
        Ok(())
    }
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid or unsupported run journal",
    )
}

fn read_run(file: &mut File) -> io::Result<Run> {
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
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
        undo_started: false,
        undo_pending: false,
        undone: 0,
        undo_finished: false,
        valid_bytes: line.len() as u64,
        torn_tail: false,
    };
    while let Some(record) = next_record(&mut reader, &mut line)? {
        run.record(record)?;
        run.valid_bytes += line.len() as u64;
    }
    run.torn_tail = !line.is_empty();
    Ok(run)
}

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

#[cfg(windows)]
pub struct RecoveryLog {
    file: File,
    pub run: Run,
}

#[cfg(windows)]
impl RecoveryLog {
    pub fn open(path: &Path, write: bool) -> io::Result<Self> {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

        let mut file = journal_options().write(write).open(path)?;
        if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::other("the journal path is a reparse point"));
        }
        let run = read_run(&mut file)?;
        Ok(Self { file, run })
    }

    pub fn resolve_apply(&mut self, applied: bool) -> io::Result<()> {
        self.append(if applied {
            Record::Renamed
        } else {
            Record::Abandoned
        })
    }

    pub fn start_undo(&mut self) -> io::Result<()> {
        self.append(Record::UndoStarted)
    }

    pub fn begin_undo(&mut self, step: usize) -> io::Result<()> {
        self.append(Record::UndoPending { step })
    }

    pub fn confirm_undo(&mut self, step: usize) -> io::Result<()> {
        self.append(Record::Undone { step })
    }

    pub fn finish_undo(&mut self) -> io::Result<()> {
        self.append(Record::UndoFinished)
    }

    fn append(&mut self, record: Record) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(self.run.valid_bytes))?;
        if self.run.torn_tail {
            self.file.set_len(self.run.valid_bytes)?;
            self.run.torn_tail = false;
        }
        append_record(&mut self.file, &record)?;
        self.run.valid_bytes = self.file.stream_position()?;
        self.run.record(record)
    }
}

#[cfg(test)]
fn inspect(path: &Path) -> io::Result<Run> {
    read_run(&mut File::open(path)?)
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
        journal.begin("before", "after", source_id(), None).unwrap();
        assert!(journal.finish().is_err());
        let run = inspect(&log).unwrap();
        assert_eq!(run.root, root_name);
        assert_eq!(run.root_identity, root_id());
        assert_eq!(run.pending.unwrap().source_identity, source_id());
        assert!(run.confirmed.is_empty());

        journal.confirm().unwrap();
        journal.finish().unwrap();
        assert!(journal.begin("before", "after", source_id(), None).is_err());
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
        journal.begin("before", "after", root_id(), None).unwrap();
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
            .begin(
                "before",
                "after",
                crate::identity::read(&source).unwrap(),
                None,
            )
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
