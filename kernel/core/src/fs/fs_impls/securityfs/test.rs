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
    security::securityfs::{self, SecurityFsDirectory, SecurityFsFile},
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
static VERSION_FILES: [SecurityFsFile; 1] = [SecurityFsFile::read_only("v5", read_version)];
static VERSION_DIRECTORIES: [SecurityFsDirectory; 1] =
    [SecurityFsDirectory::new("versions", &VERSION_FILES, &[])];
static POLICY_DIRECTORIES: [SecurityFsDirectory; 1] = [SecurityFsDirectory::new(
    "policy",
    &[],
    &VERSION_DIRECTORIES,
)];
static FEATURE_DIRECTORIES: [SecurityFsDirectory; 1] = [SecurityFsDirectory::new(
    "features",
    &[],
    &POLICY_DIRECTORIES,
)];
static NESTED_DIRECTORY: SecurityFsDirectory =
    SecurityFsDirectory::new("ktest-nested", &MODE_FILES, &FEATURE_DIRECTORIES);

static DUPLICATE_CHILDREN: [SecurityFsDirectory; 2] = [
    SecurityFsDirectory::new("same", &MODE_FILES, &[]),
    SecurityFsDirectory::new("same", &MODE_FILES, &[]),
];
static DUPLICATE_CHILD_DIRECTORY: SecurityFsDirectory =
    SecurityFsDirectory::new("ktest-duplicate-child", &[], &DUPLICATE_CHILDREN);
static FILE_DIRECTORY_COLLISION_CHILDREN: [SecurityFsDirectory; 1] =
    [SecurityFsDirectory::new("read", &MODE_FILES, &[])];
static FILE_DIRECTORY_COLLISION: SecurityFsDirectory = SecurityFsDirectory::new(
    "ktest-file-directory-collision",
    &MODE_FILES,
    &FILE_DIRECTORY_COLLISION_CHILDREN,
);

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

fn read_version(offset: usize, writer: &mut VmWriter) -> Result<usize> {
    let mut printer = aster_util::printer::VmPrinter::new_skip(writer, offset);
    printer
        .write_fmt(format_args!("yes\n"))
        .map_err(|_| Error::new(Errno::EFAULT))?;
    Ok(printer.bytes_written())
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
fn inode_ownership_is_independent_of_lookup_credentials() {
    use crate::{
        fs::utils::systree_inode::{SysTreeInodeTy, SysTreeNodeKind},
        process::{Gid, Uid},
    };

    let mut metadata = root_dentry().inode().metadata().unwrap();
    metadata.uid = Uid::new(65534);
    metadata.gid = Gid::new(65534);
    let inode: Arc<dyn crate::fs::vfs::inode::Inode> = super::inode::SecurityFsInode::new_arc(
        SysTreeNodeKind::Branch(SecurityRootNode::singleton().clone()),
        metadata,
        mkmod!(a+rx),
        Weak::new(),
    );

    let metadata = inode.metadata().unwrap();
    assert_eq!(metadata.uid, Uid::new_root());
    assert_eq!(metadata.gid, Gid::new_root());
}

#[ktest]
fn file_modes_and_missing_operations_are_fixed() {
    super::init_for_ktest();
    securityfs::register_directory("ktest-modes", &MODE_FILES).unwrap();
    let read = lookup_file("ktest-modes", "read");
    let write = lookup_file("ktest-modes", "write");

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
    let directory = SecurityDirectory::new(&SecurityFsDirectory::new(
        "ktest-missing-file",
        &MODE_FILES,
        &[],
    ))
    .unwrap();

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

#[ktest]
fn nested_directories_expose_callback_files() {
    super::init_for_ktest();
    securityfs::register_hierarchy(&NESTED_DIRECTORY).unwrap();

    let mut inode = root_dentry().inode().lookup("ktest-nested").unwrap();
    assert_eq!(inode.mode().unwrap(), mkmod!(a+rx));
    for directory in ["features", "policy", "versions"] {
        inode = inode.lookup(directory).unwrap();
        assert_eq!(inode.mode().unwrap(), mkmod!(a+rx));
    }
    let version = inode.lookup("v5").unwrap();
    assert_eq!(version.mode().unwrap(), mkmod!(a+r));

    let mut bytes = [0; 4];
    let mut writer = VmWriter::from(&mut bytes[..]).to_fallible();
    assert_eq!(
        version
            .read_at(0, &mut writer, StatusFlags::empty())
            .unwrap(),
        4
    );
    assert_eq!(&bytes, b"yes\n");

    let mut bytes = [0; 3];
    let mut writer = VmWriter::from(&mut bytes[..]).to_fallible();
    assert_eq!(
        version
            .read_at(1, &mut writer, StatusFlags::empty())
            .unwrap(),
        3
    );
    assert_eq!(&bytes, b"es\n");

    let mut bytes = [0; 1];
    let mut writer = VmWriter::from(&mut bytes[..]).to_fallible();
    assert_eq!(
        version
            .read_at(4, &mut writer, StatusFlags::empty())
            .unwrap(),
        0
    );
}

#[ktest]
fn nested_directory_sibling_names_are_unique() {
    super::init_for_ktest();
    assert_errno(
        securityfs::register_hierarchy(&DUPLICATE_CHILD_DIRECTORY),
        Errno::EEXIST,
    );
    assert_errno(
        securityfs::register_hierarchy(&FILE_DIRECTORY_COLLISION),
        Errno::EEXIST,
    );
}
