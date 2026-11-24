use std::{io, path::Path, process::ExitCode};

use crate::{
    directory::display,
    identity,
    journal::{RecoveryLog, Run, Step},
    windows_rename::{self, RenameLocation},
};

#[derive(Debug, PartialEq, Eq)]
enum Position {
    Original,
    Renamed { original_free: bool },
}

pub fn run(path: &Path, undo: bool) -> ExitCode {
    let mut journal = match RecoveryLog::open(path, undo) {
        Ok(journal) => journal,
        Err(error) => {
            eprintln!(
                "Cannot open run journal {}: {error}",
                display(path.as_os_str())
            );
            return ExitCode::from(2);
        }
    };
    let result = if undo {
        undo_run(&mut journal)
    } else {
        inspect(&journal.run)
    };
    if let Err(error) = result {
        eprintln!(
            "Run needs review after {} confirmed undo step(s): {error}",
            journal.run.undone
        );
        return ExitCode::from(3);
    }
    ExitCode::SUCCESS
}

fn validate_root(run: &Run) -> io::Result<()> {
    let root = Path::new(&run.root);
    if !root.is_absolute()
        || !std::fs::symlink_metadata(root)?.is_dir()
        || identity::read(root)? != run.root_identity
    {
        return Err(io::Error::other(
            "the recorded root directory was replaced or is invalid",
        ));
    }
    Ok(())
}

fn locate<'a>(run: &Run, step: &'a Step) -> io::Result<RenameLocation<'a>> {
    windows_rename::locate(
        Path::new(&run.root),
        &run.root_identity,
        step.parent_identity.as_ref(),
        &step.source,
        &step.target,
    )
    .map_err(|error| step_error(step, error))
}

fn step_error(step: &Step, error: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("{:?} -> {:?}: {error}", step.source, step.target))
}

fn position(location: &RenameLocation<'_>, step: &Step) -> io::Result<Position> {
    let (original, renamed) = location
        .identities()
        .map_err(|error| step_error(step, error))?;
    match (
        original.as_ref() == Some(&step.source_identity),
        renamed.as_ref() == Some(&step.source_identity),
    ) {
        (true, false) => Ok(Position::Original),
        (false, true) => Ok(Position::Renamed {
            original_free: original.is_none(),
        }),
        (true, true) => Err(step_error(step, "both names refer to the recorded entry")),
        (false, false) => Err(step_error(
            step,
            "the recorded entry was moved, removed or replaced",
        )),
    }
}

fn inspect(run: &Run) -> io::Result<()> {
    let status = if run.undo_finished {
        "Undo completed"
    } else if run.undo_started {
        "Undo interrupted"
    } else if run.finished {
        "Apply completed"
    } else {
        "Apply interrupted"
    };
    println!(
        "{status}. {} confirmed rename(s), {} undone.",
        run.confirmed.len(),
        run.undone
    );
    if run.torn_tail {
        println!("The journal ends with an incomplete record.");
    }
    if run.undo_finished {
        return Ok(());
    }
    validate_root(run)?;
    if let Some(step) = &run.pending {
        let location = locate(run, step)?;
        match position(&location, step)? {
            Position::Original => {
                println!("Pending apply step was not performed: {:?}.", step.source)
            }
            Position::Renamed { .. } => {
                println!("Pending apply step was performed: {:?}.", step.target)
            }
        }
    } else if run.undo_pending
        && let Some((number, step)) = run.next_undo()
    {
        let location = locate(run, step)?;
        match position(&location, step)? {
            Position::Original => println!(
                "Pending undo step {number} was performed: {:?}.",
                step.source
            ),
            Position::Renamed {
                original_free: true,
            } => println!(
                "Pending undo step {number} can be retried: {:?}.",
                step.target
            ),
            Position::Renamed {
                original_free: false,
            } => return Err(step_error(step, "the original name is occupied")),
        }
    }
    Ok(())
}

fn undo_run(journal: &mut RecoveryLog) -> io::Result<()> {
    if journal.run.undo_finished {
        println!("This run has already been undone.");
        return Ok(());
    }
    validate_root(&journal.run)?;
    if let Some(step) = &journal.run.pending {
        let location = locate(&journal.run, step)?;
        let applied = matches!(position(&location, step)?, Position::Renamed { .. });
        journal.resolve_apply(applied)?;
        println!("Resolved the pending apply step.");
    }
    if !journal.run.undo_started {
        journal.start_undo()?;
    }

    while let Some((number, step)) = journal
        .run
        .next_undo()
        .map(|(number, step)| (number, step.clone()))
    {
        let location = locate(&journal.run, &step)?;
        match position(&location, &step)? {
            Position::Original if journal.run.undo_pending => {}
            Position::Original => {
                return Err(step_error(
                    &step,
                    "the original name already contains the entry",
                ));
            }
            Position::Renamed {
                original_free: false,
            } => return Err(step_error(&step, "the original name is occupied")),
            Position::Renamed {
                original_free: true,
            } => {
                if !journal.run.undo_pending {
                    journal.begin_undo(number)?;
                }
                location
                    .restore(&step.source_identity)
                    .map_err(|error| step_error(&step, error))?;
            }
        }
        journal.confirm_undo(number)?;
        println!("Restored {:?}.", step.source);
    }
    journal.finish_undo()?;
    println!("Undo completed. {} rename(s) reversed.", journal.run.undone);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::Journal;
    use std::{fs, io::Write};

    #[test]
    fn resolves_unconfirmed_apply_with_or_without_the_rename() {
        for renamed in [false, true] {
            let workspace = tempfile::tempdir().unwrap();
            let root = workspace.path();
            let path = root.join("run.jsonl");
            let root_id = identity::read(root).unwrap();
            fs::write(root.join("before"), b"original").unwrap();
            let source_id = identity::read(&root.join("before")).unwrap();
            let mut forward =
                Journal::create(&path, root.to_str().unwrap(), root_id.clone()).unwrap();
            forward
                .begin("before", "after", source_id.clone(), Some(root_id.clone()))
                .unwrap();
            assert!(
                RecoveryLog::open(&path, false).is_err(),
                "the apply writer is still active"
            );
            if renamed {
                windows_rename::rename(root, &root_id, "before", &source_id, "after").unwrap();
            } else {
                fs::write(root.join("after"), b"keep").unwrap();
            }
            drop(forward);
            fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap()
                .write_all(b"{\"event\":")
                .unwrap();
            let saved = fs::read(&path).unwrap();
            assert_eq!(run(&path, false), ExitCode::SUCCESS);
            assert_eq!(fs::read(&path).unwrap(), saved);
            assert_eq!(run(&path, true), ExitCode::SUCCESS);
            let journal = RecoveryLog::open(&path, false).unwrap();
            assert_eq!(journal.run.undone, usize::from(renamed));
            assert!(journal.run.undo_finished);
            assert!(!journal.run.torn_tail);
            assert_eq!(fs::read(root.join("before")).unwrap(), b"original");
            if renamed {
                assert!(!root.join("after").exists());
            } else {
                assert_eq!(fs::read(root.join("after")).unwrap(), b"keep");
            }
        }
    }

    #[test]
    fn retries_pending_undo_and_confirms_a_rename_already_reversed() {
        for restored in [false, true] {
            let workspace = tempfile::tempdir().unwrap();
            let root = workspace.path();
            let path = root.join("run.jsonl");
            let root_id = identity::read(root).unwrap();
            fs::write(root.join("before"), b"original").unwrap();
            let source_id = identity::read(&root.join("before")).unwrap();
            let mut forward =
                Journal::create(&path, root.to_str().unwrap(), root_id.clone()).unwrap();
            // Older runs did not include a parent identity.
            forward
                .begin("before", "after", source_id.clone(), None)
                .unwrap();
            windows_rename::rename(root, &root_id, "before", &source_id, "after").unwrap();
            forward.confirm().unwrap();
            forward.finish().unwrap();
            drop(forward);

            let mut journal = RecoveryLog::open(&path, true).unwrap();
            journal.start_undo().unwrap();
            journal.begin_undo(1).unwrap();
            if restored {
                windows_rename::rename(root, &root_id, "after", &source_id, "before").unwrap();
            } else {
                fs::write(root.join("before"), b"occupied").unwrap();
            }
            drop(journal);
            if !restored {
                assert_eq!(run(&path, true), ExitCode::from(3));
                assert_eq!(fs::read(root.join("before")).unwrap(), b"occupied");
                assert_eq!(fs::read(root.join("after")).unwrap(), b"original");
                fs::remove_file(root.join("before")).unwrap();
            }
            assert_eq!(run(&path, true), ExitCode::SUCCESS);
            assert_eq!(fs::read(root.join("before")).unwrap(), b"original");
            assert!(!root.join("after").exists());
            assert_eq!(RecoveryLog::open(&path, false).unwrap().run.undone, 1);
        }
    }

    #[test]
    fn refuses_to_undo_a_replaced_entry() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path();
        let path = root.join("run.jsonl");
        let root_id = identity::read(root).unwrap();
        fs::write(root.join("before"), b"original").unwrap();
        let source_id = identity::read(&root.join("before")).unwrap();
        let mut forward = Journal::create(&path, root.to_str().unwrap(), root_id.clone()).unwrap();
        forward
            .begin("before", "after", source_id.clone(), Some(root_id.clone()))
            .unwrap();
        windows_rename::rename(root, &root_id, "before", &source_id, "after").unwrap();
        forward.confirm().unwrap();
        forward.finish().unwrap();
        drop(forward);
        fs::rename(root.join("after"), root.join("moved")).unwrap();
        fs::write(root.join("after"), b"replacement").unwrap();
        assert_eq!(run(&path, true), ExitCode::from(3));
        assert!(!root.join("before").exists());
        assert_eq!(fs::read(root.join("after")).unwrap(), b"replacement");
        assert_eq!(fs::read(root.join("moved")).unwrap(), b"original");
    }
}
