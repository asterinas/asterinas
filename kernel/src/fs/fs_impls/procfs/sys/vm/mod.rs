// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        file::{InodeType, mkmod},
        procfs::{
            ProcDir, StaticEntry,
            template::{
                ProcDirOps, ProcFile, ProcFileOps, ReaddirEntry, listed_entries_from_table,
                lookup_child_from_table, visit_listed_entries,
            },
        },
        vfs::inode::Inode,
    },
    prelude::*,
};

const MAX_MAP_COUNT: usize = 1_048_576;
const MMAP_MIN_ADDR: usize = 65_536;
const OVERCOMMIT_MEMORY: usize = 0;

/// Represents the inode at `/proc/sys/vm`.
pub struct VmDirOps;

impl VmDirOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/mm/util.c>
        ProcDir::new(Self, parent, mkmod!(a+rx))
    }

    const STATIC_ENTRIES: &'static [StaticEntry] = &[
        (
            "max_map_count",
            InodeType::File,
            MaxMapCountFileOps::new_inode,
        ),
        (
            "mmap_min_addr",
            InodeType::File,
            MmapMinAddrFileOps::new_inode,
        ),
        (
            "overcommit_memory",
            InodeType::File,
            OvercommitMemoryFileOps::new_inode,
        ),
    ];
}

impl ProcDirOps for VmDirOps {
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

struct MaxMapCountFileOps;

impl MaxMapCountFileOps {
    fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ReadOnlyValueFileOps::new_inode(parent, "max_map_count", MAX_MAP_COUNT)
    }
}

struct MmapMinAddrFileOps;

impl MmapMinAddrFileOps {
    fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ReadOnlyValueFileOps::new_inode(parent, "mmap_min_addr", MMAP_MIN_ADDR)
    }
}

struct OvercommitMemoryFileOps;

impl OvercommitMemoryFileOps {
    fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ReadOnlyValueFileOps::new_inode(parent, "overcommit_memory", OVERCOMMIT_MEMORY)
    }
}

struct ReadOnlyValueFileOps {
    name: &'static str,
    value: usize,
}

impl ReadOnlyValueFileOps {
    fn new_inode(parent: Weak<dyn Inode>, name: &'static str, value: usize) -> Arc<dyn Inode> {
        ProcFile::new(Self { name, value }, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for ReadOnlyValueFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        writeln!(printer, "{}", self.value)?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, _reader: &mut VmReader) -> Result<usize> {
        warn!("writing to `/proc/sys/vm/{}` is not supported", self.name);
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "writing to the `/proc/sys/vm` file is not supported"
        );
    }
}
