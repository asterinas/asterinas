// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        file::{InodeType, mkmod},
        procfs::template::{
            DirOps, FileOps, ProcDir, ProcFile, ReaddirEntry, StaticDirEntry,
            listed_entries_from_table, lookup_child_from_table, read_i32_from,
            visit_listed_entries,
        },
        vfs::inode::Inode,
    },
    prelude::*,
    process::posix_thread::alien_access::yama::{YamaScope, get_yama_scope, set_yama_scope},
};

/// Represents the inode at `/proc/sys/kernel/yama`.
pub struct YamaDirOps;

impl YamaDirOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference:
        // <https://elixir.bootlin.com/linux/v6.16.5/source/security/yama/yama_lsm.c#L463>
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/proc_sysctl.c#L978>
        ProcDir::new(Self, parent, mkmod!(a+rx))
    }

    #[expect(clippy::type_complexity)]
    const STATIC_ENTRIES: &'static [StaticDirEntry<fn(Weak<dyn Inode>) -> Arc<dyn Inode>>] = &[(
        "ptrace_scope",
        InodeType::File,
        PtraceScopeFileOps::new_inode,
    )];
}

impl DirOps for YamaDirOps {
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

/// Represents the inode at `/proc/sys/kernel/yama/ptrace_scope`.
struct PtraceScopeFileOps;

impl PtraceScopeFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/security/yama/yama_lsm.c#L455>
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl FileOps for PtraceScopeFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        writeln!(printer, "{}", get_yama_scope() as i32)?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        let (val, read_bytes) = read_i32_from(reader)?;
        let new_scope = YamaScope::try_from(val)?;

        set_yama_scope(new_scope)?;

        Ok(read_bytes)
    }
}
