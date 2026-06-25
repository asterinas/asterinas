// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicI32, Ordering};

use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        file::{InodeType, mkmod},
        procfs::{
            StaticEntry,
            template::{
                ProcDir, ProcDirOps, ProcFile, ProcFileOps, ReaddirEntry,
                listed_entries_from_table, lookup_child_from_table, read_i32_from,
                visit_listed_entries,
            },
        },
        vfs::inode::Inode,
    },
    prelude::*,
};

const DEFAULT_MAXKEYS: i32 = 200;

static MAXKEYS: AtomicI32 = AtomicI32::new(DEFAULT_MAXKEYS);

/// Represents the inode at `/proc/sys/kernel/keys`.
pub struct KeysDirOps;

impl KeysDirOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/security/keys/sysctl.c>
        ProcDir::new(Self, parent, mkmod!(a+rx))
    }

    const STATIC_ENTRIES: &'static [StaticEntry] =
        &[("maxkeys", InodeType::File, MaxKeysFileOps::new_inode)];
}

impl ProcDirOps for KeysDirOps {
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

/// Represents the inode at `/proc/sys/kernel/keys/maxkeys`.
struct MaxKeysFileOps;

impl MaxKeysFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for MaxKeysFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        writeln!(printer, "{}", MAXKEYS.load(Ordering::Relaxed))?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        let (val, read_bytes) = read_i32_from(reader)?;
        if val < 0 {
            return_errno_with_message!(Errno::EINVAL, "the maxkeys value cannot be negative");
        }

        MAXKEYS.store(val, Ordering::Relaxed);

        Ok(read_bytes)
    }
}
