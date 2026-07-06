// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        file::{InodeType, mkmod},
        procfs::{
            StaticEntry,
            template::{
                ProcDir, ProcDirOps, ProcFile, ProcFileOps, ReaddirEntry,
                listed_entries_from_table, lookup_child_from_table, visit_listed_entries,
            },
        },
        vfs::inode::Inode,
    },
    prelude::*,
    security,
};

const MAX_POLICY_WRITE_LEN: usize = 64 * 1024;

/// Represents the inode at `/proc/smack`.
pub struct SmackDirOps;

impl SmackDirOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcDir::new(Self, parent, mkmod!(a+rx))
    }

    const STATIC_ENTRIES: &'static [StaticEntry] = &[
        ("load", InodeType::File, LoadFileOps::new_inode),
        ("accesses", InodeType::File, AccessesFileOps::new_inode),
    ];
}

impl ProcDirOps for SmackDirOps {
    fn lookup_child(&self, this_dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if let Some(child) = lookup_child_from_table(name, Self::STATIC_ENTRIES, |f| {
            (f)(this_dir.this_weak().clone())
        }) {
            return Ok(child);
        }

        return_errno_with_message!(Errno::ENOENT, "the file does not exist");
    }

    fn visit_entries_from_offset<'a, F>(&'a self, offset: usize, visit_fn: F) -> Result<()>
    where
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        visit_listed_entries(
            offset,
            listed_entries_from_table(Self::STATIC_ENTRIES),
            visit_fn,
        )
    }
}

/// Represents the inode at `/proc/smack/load`.
struct LoadFileOps;

impl LoadFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for LoadFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        write_rules_at(offset, writer)
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        read_and_load_rules(reader)
    }
}

/// Represents the inode at `/proc/smack/accesses`.
struct AccessesFileOps;

impl AccessesFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r))
    }
}

impl ProcFileOps for AccessesFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        write_rules_at(offset, writer)
    }
}

fn write_rules_at(offset: usize, writer: &mut VmWriter) -> Result<usize> {
    let rules = security::smack_rules_as_text()?;
    let mut printer = VmPrinter::new_skip(writer, offset);
    write!(printer, "{}", rules)?;
    Ok(printer.bytes_written())
}

fn read_and_load_rules(reader: &mut VmReader) -> Result<usize> {
    let read_bytes = reader.remain();
    if read_bytes > MAX_POLICY_WRITE_LEN {
        return_errno_with_message!(Errno::E2BIG, "the Smack policy write is too large");
    }

    let mut policy = vec![0u8; read_bytes];
    reader.read_fallible(&mut VmWriter::from(policy.as_mut_slice()))?;
    let policy = core::str::from_utf8(&policy)
        .map_err(|_| Error::with_message(Errno::EINVAL, "the Smack policy is not UTF-8"))?;
    security::load_smack_rules(policy)?;

    Ok(read_bytes)
}
