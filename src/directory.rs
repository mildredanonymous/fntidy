use std::{
    ffi::OsStr,
    fs,
    path::{Component, Path, PathBuf},
};

use crate::windows_names;

pub struct Entry {
    pub key: Vec<u16>,
    pub name: String,
    pub redirected: bool,
    is_dir: bool,
}

pub struct Directory {
    pub entries: Vec<Entry>,
    pub incomplete: bool,
}

pub fn walk(root: &Path, recursive: bool, mut visit: impl FnMut(&Path, Directory)) -> Option<bool> {
    let first = scan(root)?;
    if !recursive {
        let incomplete = first.incomplete;
        visit(Path::new(""), first);
        return Some(incomplete);
    }

    let base = match root.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("Cannot resolve {}: {error}", display(root.as_os_str()));
            visit(Path::new(""), first);
            return Some(true);
        }
    };
    let mut pending = Vec::new();
    let mut incomplete = false;
    let mut visit_directory = |parent: &Path, directory: Directory, pending: &mut Vec<PathBuf>| {
        let mut incomplete = directory.incomplete;
        for entry in directory
            .entries
            .iter()
            .rev()
            .filter(|entry| entry.is_dir && !entry.redirected)
        {
            if is_name_component(&entry.name) {
                pending.push(parent.join(&entry.name));
            } else {
                eprintln!(
                    "Cannot descend into {}: ambiguous directory name.",
                    display(OsStr::new(&entry.name))
                );
                incomplete = true;
            }
        }
        visit(parent, directory);
        incomplete
    };
    incomplete |= visit_directory(Path::new(""), first, &mut pending);
    while let Some(parent) = pending.pop() {
        match scan(&base.join(&parent)) {
            Some(directory) => incomplete |= visit_directory(&parent, directory, &mut pending),
            None => incomplete = true,
        }
    }
    Some(incomplete)
}

pub fn is_name_component(name: &str) -> bool {
    matches!(Path::new(name).components().next(), Some(Component::Normal(component)) if component == name)
        && !name.contains('\0')
        && !(cfg!(windows) && name.contains(':'))
}

fn scan(root: &Path) -> Option<Directory> {
    let root = root.components().collect::<std::path::PathBuf>();
    let root = root.as_path();
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) => {
            eprintln!("Cannot inspect {}: {error}", display(root.as_os_str()));
            return None;
        }
    };
    if redirected(&metadata) || !metadata.is_dir() {
        eprintln!(
            "Expected a directory without redirection: {}",
            display(root.as_os_str())
        );
        return None;
    }
    let directory = match fs::read_dir(root) {
        Ok(directory) => directory,
        Err(error) => {
            eprintln!("Cannot read {}: {error}", display(root.as_os_str()));
            return None;
        }
    };

    let mut entries = Vec::new();
    let mut incomplete = false;
    for item in directory {
        let entry = match item {
            Ok(entry) => entry,
            Err(error) => {
                eprintln!("Cannot read an entry: {error}");
                incomplete = true;
                continue;
            }
        };
        let native_name = entry.file_name();
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                eprintln!("Cannot inspect {}: {error}", display(&native_name));
                incomplete = true;
                continue;
            }
        };
        let Some(name) = native_name.into_string().ok() else {
            eprintln!("Skipped an entry whose name cannot be represented as Unicode.");
            incomplete = true;
            continue;
        };
        let redirected = redirected(&metadata);
        if redirected {
            eprintln!("Skipped redirected entry {}.", display(OsStr::new(&name)));
        }
        // Redirected entries still occupy a name in the same directory.
        entries.push(Entry {
            key: windows_names::comparison_units(&name),
            name,
            redirected,
            is_dir: metadata.is_dir(),
        });
    }

    entries.sort_unstable_by(|left, right| {
        windows_names::compare(&left.key, &right.key).then_with(|| left.name.cmp(&right.name))
    });
    Some(Directory {
        entries,
        incomplete,
    })
}

fn redirected(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    metadata.file_type().is_symlink()
}

pub fn display(value: &OsStr) -> String {
    format!("{value:?}")
}
