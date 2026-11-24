use std::{
    fs::File,
    io, mem,
    os::windows::io::AsRawHandle,
    path::{MAIN_SEPARATOR, Path},
    ptr,
};

use windows_sys::{
    Wdk::Storage::FileSystem::{FileRenameInformation, NtSetInformationFile},
    Win32::{
        Foundation::RtlNtStatusToDosError,
        Storage::FileSystem::{DELETE, FILE_READ_ATTRIBUTES, FILE_RENAME_INFO},
        System::IO::IO_STATUS_BLOCK,
    },
};

use crate::{
    directory::is_name_component,
    identity::{self, Identity},
    windows_names,
};

pub fn rename_from_root(
    root: &Path,
    expected_root: &Identity,
    expected_parent: &Identity,
    source: &str,
    expected_source: &Identity,
    target: &str,
) -> io::Result<()> {
    if source.starts_with(MAIN_SEPARATOR) || target.starts_with(MAIN_SEPARATOR) {
        return Err(io::Error::other("rename paths must be relative"));
    }
    let (parent, source) = source.rsplit_once(MAIN_SEPARATOR).unwrap_or(("", source));
    let (target_parent, target) = target.rsplit_once(MAIN_SEPARATOR).unwrap_or(("", target));
    if parent != target_parent
        || !is_name_component(source)
        || !is_name_component(target)
        || target.encode_utf16().count() > 255
        || windows_names::violations(target).next().is_some()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid rename component",
        ));
    }

    // Hold each directory without delete sharing while resolving the nested path.
    let root_handle = identity::open(root, FILE_READ_ATTRIBUTES, 0)?;
    if !root_handle.metadata()?.is_dir() || identity::from_handle(&root_handle)? != *expected_root {
        return Err(io::Error::other("the directory was replaced"));
    }
    let mut base = root.canonicalize()?;
    if identity::read(&base)? != *expected_root {
        return Err(io::Error::other("the directory path changed"));
    }
    let mut directories = vec![root_handle];
    if !parent.is_empty() {
        for component in parent.split(MAIN_SEPARATOR) {
            if !is_name_component(component) {
                return Err(io::Error::other("invalid parent path component"));
            }
            base.push(component);
            let directory = identity::open(&base, FILE_READ_ATTRIBUTES, 0)?;
            if !directory.metadata()?.is_dir() {
                return Err(io::Error::other("the parent path contains a non-directory"));
            }
            identity::from_handle(&directory)?;
            directories.push(directory);
        }
    }
    let directory = directories.last().expect("the root directory is open");
    if identity::from_handle(directory)? != *expected_parent {
        return Err(io::Error::other("the parent directory was replaced"));
    }

    let entry = identity::open(&base.join(source), DELETE | FILE_READ_ATTRIBUTES, 0)?;
    if identity::from_handle(&entry)? != *expected_source {
        return Err(io::Error::other("the source was replaced"));
    }
    if identity::read(&base)? != *expected_parent {
        return Err(io::Error::other("the directory path changed"));
    }
    rename_handle(directory, &entry, target)
}

#[cfg(test)]
pub fn rename(
    root: &Path,
    expected_root: &Identity,
    source: &str,
    expected_source: &Identity,
    target: &str,
) -> io::Result<()> {
    rename_from_root(
        root,
        expected_root,
        expected_root,
        source,
        expected_source,
        target,
    )
}

fn rename_handle(directory: &File, entry: &File, target: &str) -> io::Result<()> {
    let name = target.encode_utf16().collect::<Vec<_>>();
    let name_bytes = name.len() * mem::size_of::<u16>();
    let name_offset = mem::offset_of!(FILE_RENAME_INFO, FileName);
    let buffer_size =
        (name_offset + name_bytes + mem::size_of::<u16>()).max(mem::size_of::<FILE_RENAME_INFO>());
    // Pointer-sized storage supplies the structure's alignment and space for its trailing name.
    let mut buffer = vec![0usize; buffer_size.div_ceil(mem::size_of::<usize>())];
    let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();

    let mut io_status = IO_STATUS_BLOCK::default();
    // SAFETY: The aligned, zero-initialized buffer includes the header and full UTF-16 name.
    // Both file handles stay open for the duration of the call. The zero flags forbid replacement.
    let status = unsafe {
        (*info).RootDirectory = directory.as_raw_handle();
        (*info).FileNameLength = name_bytes as u32;
        ptr::copy_nonoverlapping(
            name.as_ptr(),
            ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
            name.len(),
        );
        NtSetInformationFile(
            entry.as_raw_handle(),
            &mut io_status,
            info.cast(),
            buffer_size as u32,
            FileRenameInformation,
        )
    };

    if status == 0 {
        Ok(())
    } else {
        // SAFETY: The conversion accepts any NTSTATUS value.
        Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(status) } as i32,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn renames_the_verified_entry_in_its_directory() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().canonicalize().unwrap().join("before.txt.");
        fs::write(&source, b"original").unwrap();
        let root_id = identity::read(root.path()).unwrap();
        let source_id = identity::read(&source).unwrap();

        rename(root.path(), &root_id, "before.txt.", &source_id, "März.txt").unwrap();

        let target = root.path().join("März.txt");
        assert_eq!(fs::read(&target).unwrap(), b"original");
        assert_eq!(identity::read(&target).unwrap(), source_id);
        assert!(!source.exists());
    }

    #[test]
    fn refuses_an_occupied_target_and_a_replaced_source() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("before.txt");
        let target = root.path().join("after.txt");
        fs::write(&source, b"original").unwrap();
        let root_id = identity::read(root.path()).unwrap();
        let source_id = identity::read(&source).unwrap();

        fs::write(&target, b"occupied").unwrap();
        assert!(rename(root.path(), &root_id, "before.txt", &source_id, "after.txt").is_err());
        assert_eq!(fs::read(&target).unwrap(), b"occupied");
        assert_eq!(fs::read(&source).unwrap(), b"original");

        fs::remove_file(&target).unwrap();
        fs::rename(&source, root.path().join("previous.txt")).unwrap();
        fs::write(&source, b"replacement").unwrap();
        let error =
            rename(root.path(), &root_id, "before.txt", &source_id, "after.txt").unwrap_err();
        assert!(error.to_string().contains("source was replaced"));
        assert_eq!(fs::read(&source).unwrap(), b"replacement");
        assert!(!target.exists());
    }
}
