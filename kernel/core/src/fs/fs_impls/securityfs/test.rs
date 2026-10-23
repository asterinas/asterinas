// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicUsize, Ordering};

use aster_systree::SysObj;
use ostd::prelude::ktest;

use super::{
    fs::SecurityFs,
    node::{SecurityDirectory, SecurityRootNode},
};
use crate::{
    fs::{
        file::{InodeType, StatusFlags, mkmod},
        vfs::{file_system::FileSystem, path::Dentry},
    },
    prelude::*,
    security::securityfs::{self, SecurityFsFile},
};

static READ_OFFSET: AtomicUsize = AtomicUsize::new(usize::MAX);

fn read_empty(_offset: usize, _writer: &mut VmWriter) -> Result<usize> {
    Ok(0)
}

fn read_offset(offset: usize, _writer: &mut VmWriter) -> Result<usize> {
    READ_OFFSET.store(offset, Ordering::Relaxed);
    Ok(0)
}

fn write_empty(_offset: usize, _reader: &mut VmReader) -> Result<usize> {
    Ok(0)
}

fn write_eproto(_offset: usize, _reader: &mut VmReader) -> Result<usize> {
    Err(Error::new(Errno::EPROTO))
}

static MODE_FILES: [SecurityFsFile; 2] = [
    SecurityFsFile::read_only("read", read_empty),
    SecurityFsFile::write_only("write", write_empty),
];
static OFFSET_FILES: [SecurityFsFile; 1] = [SecurityFsFile::read_only("offset", read_offset)];
static EPROTO_FILES: [SecurityFsFile; 1] = [SecurityFsFile::write_only("load", write_eproto)];
static EMPTY_NAME_FILES: [SecurityFsFile; 1] = [SecurityFsFile::read_only("", read_empty)];
static DOT_NAME_FILES: [SecurityFsFile; 1] = [SecurityFsFile::read_only(".", read_empty)];
static SLASH_NAME_FILES: [SecurityFsFile; 1] = [SecurityFsFile::read_only("bad/name", read_empty)];
static NUL_NAME_FILES: [SecurityFsFile; 1] = [SecurityFsFile::read_only("bad\0name", read_empty)];
static DUPLICATE_FILES: [SecurityFsFile; 2] = [
    SecurityFsFile::read_only("same", read_empty),
    SecurityFsFile::write_only("same", write_empty),
];
static CREATE_FILES: [SecurityFsFile; 1] = [SecurityFsFile::read_only("status", read_empty)];

const TOO_LONG_NAME: &str = concat!(
    "aaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaa",
);
static LONG_NAME_FILES: [SecurityFsFile; 1] =
    [SecurityFsFile::read_only(TOO_LONG_NAME, read_empty)];

fn assert_errno<T>(result: Result<T>, expected: Errno) {
    assert_eq!(result.err().unwrap().error(), expected);
}

fn root_dentry() -> Arc<Dentry> {
    super::init_for_ktest();
    Dentry::new_root(SecurityFs::singleton().root_inode())
}

fn lookup_file(directory: &str, file: &str) -> Arc<dyn crate::fs::vfs::inode::Inode> {
    root_dentry()
        .inode()
        .lookup(directory)
        .unwrap()
        .lookup(file)
        .unwrap()
}

#[ktest]
fn file_modes_and_missing_operations_are_fixed() {
    super::init_for_ktest();
    securityfs::register_directory("ktest-modes", &MODE_FILES).unwrap();
    let directory = root_dentry().inode().lookup("ktest-modes").unwrap();
    let read = lookup_file("ktest-modes", "read");
    let write = lookup_file("ktest-modes", "write");

    assert_eq!(directory.mode().unwrap(), mkmod!(a+rx));
    assert_eq!(read.mode().unwrap(), mkmod!(a+r));
    assert_eq!(write.mode().unwrap(), mkmod!(a+rw));

    let mut reader = VmReader::from(&[][..]).to_fallible();
    assert_errno(
        read.write_at(0, &mut reader, StatusFlags::empty()),
        Errno::EBADF,
    );
    let mut bytes = [];
    let mut writer = VmWriter::from(&mut bytes[..]).to_fallible();
    assert_errno(
        write.read_at(0, &mut writer, StatusFlags::empty()),
        Errno::EBADF,
    );
}

#[ktest]
fn invalid_names_empty_files_and_duplicates_are_rejected() {
    super::init_for_ktest();

    for name in ["", ".", "..", "bad/name", "bad\0name", TOO_LONG_NAME] {
        assert_errno(
            securityfs::register_directory(name, &MODE_FILES),
            Errno::EINVAL,
        );
    }
    assert_errno(
        securityfs::register_directory("ktest-empty-files", &[]),
        Errno::EINVAL,
    );
    assert_errno(
        securityfs::register_directory("ktest-empty-file-name", &EMPTY_NAME_FILES),
        Errno::EINVAL,
    );
    assert_errno(
        securityfs::register_directory("ktest-dot-file-name", &DOT_NAME_FILES),
        Errno::EINVAL,
    );
    assert_errno(
        securityfs::register_directory("ktest-slash-file-name", &SLASH_NAME_FILES),
        Errno::EINVAL,
    );
    assert_errno(
        securityfs::register_directory("ktest-nul-file-name", &NUL_NAME_FILES),
        Errno::EINVAL,
    );
    assert_errno(
        securityfs::register_directory("ktest-long-file-name", &LONG_NAME_FILES),
        Errno::EINVAL,
    );
    assert_errno(
        securityfs::register_directory("ktest-duplicate-files", &DUPLICATE_FILES),
        Errno::EEXIST,
    );

    securityfs::register_directory("ktest-duplicate-directory", &MODE_FILES).unwrap();
    assert_errno(
        securityfs::register_directory("ktest-duplicate-directory", &MODE_FILES),
        Errno::EEXIST,
    );
}

#[ktest]
fn missing_callback_file_returns_enoent() {
    let directory = SecurityDirectory::new("ktest-missing-file", &MODE_FILES).unwrap();

    let mut bytes = [];
    let mut writer = VmWriter::from(&mut bytes[..]).to_fallible();
    assert_errno(
        directory.read_file_at("missing", 0, &mut writer),
        Errno::ENOENT,
    );

    let mut reader = VmReader::from(&[][..]).to_fallible();
    assert_errno(
        directory.write_file_at("missing", 0, &mut reader),
        Errno::ENOENT,
    );
}

#[ktest]
fn callback_errno_and_read_offset_survive_vfs_io() {
    super::init_for_ktest();
    securityfs::register_directory("ktest-eproto", &EPROTO_FILES).unwrap();
    securityfs::register_directory("ktest-offset", &OFFSET_FILES).unwrap();

    let load = lookup_file("ktest-eproto", "load");
    let mut reader = VmReader::from(&[][..]).to_fallible();
    assert_errno(
        load.write_at(0, &mut reader, StatusFlags::empty()),
        Errno::EPROTO,
    );

    let offset = lookup_file("ktest-offset", "offset");
    let mut bytes = [];
    let mut writer = VmWriter::from(&mut bytes[..]).to_fallible();
    offset
        .read_at(37, &mut writer, StatusFlags::empty())
        .unwrap();
    assert_eq!(READ_OFFSET.load(Ordering::Relaxed), 37);
}

#[ktest]
fn root_is_parentless_and_userspace_cannot_mutate_tree() {
    super::init_for_ktest();
    securityfs::register_directory("ktest-mutation", &CREATE_FILES).unwrap();
    let root_node = SecurityRootNode::singleton();
    assert!(root_node.is_root());
    assert!(root_node.parent().is_none());

    let root = root_dentry();
    assert_errno(
        root.inode()
            .create(&root, "new", InodeType::Dir, mkmod!(a+rx)),
        Errno::EPERM,
    );
    let child = root
        .as_dir_dentry_or_err()
        .unwrap()
        .lookup_child("ktest-mutation")
        .unwrap();
    assert_errno(root.inode().unlink(&child), Errno::EPERM);
}
