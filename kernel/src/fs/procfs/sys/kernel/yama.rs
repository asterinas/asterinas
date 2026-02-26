// SPDX-License-Identifier: MPL-2.0

use aster_util::{printer::VmPrinter, slot_vec::SlotVec};
use ostd::sync::RwMutexUpgradeableGuard;

use crate::{
    fs::{
        procfs::template::{
            DirOps, FileOps, ProcDir, ProcDirBuilder, ProcFileBuilder, lookup_child_from_table,
            populate_children_from_table,
        },
        utils::{Inode, mkmod},
    },
    prelude::*,
};

/// Represents the inode at `/proc/sys/kernel/yama`.
pub struct YamaDirOps;

impl YamaDirOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference:
        // <https://elixir.bootlin.com/linux/v6.16.5/source/security/yama/yama_lsm.c#L463>
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/proc_sysctl.c#L978>
        ProcDirBuilder::new(Self, mkmod!(a+rx))
            .parent(parent)
            .build()
            .unwrap()
    }

    #[expect(clippy::type_complexity)]
    const STATIC_ENTRIES: &'static [(&'static str, fn(Weak<dyn Inode>) -> Arc<dyn Inode>)] =
        &[("ptrace_scope", PtraceScopeFileOps::new_inode)];
}

impl DirOps for YamaDirOps {
    fn lookup_child(&self, dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        let mut cached_children = dir.cached_children().write();

        if let Some(child) =
            lookup_child_from_table(name, &mut cached_children, Self::STATIC_ENTRIES, |f| {
                (f)(dir.this_weak().clone())
            })
        {
            return Ok(child);
        }

        return_errno_with_message!(Errno::ENOENT, "the file does not exist");
    }

    fn populate_children<'a>(
        &self,
        dir: &'a ProcDir<Self>,
    ) -> RwMutexUpgradeableGuard<'a, SlotVec<(String, Arc<dyn Inode>)>> {
        let mut cached_children = dir.cached_children().write();

        populate_children_from_table(&mut cached_children, Self::STATIC_ENTRIES, |f| {
            (f)(dir.this_weak().clone())
        });

        cached_children.downgrade()
    }
}

/// Represents the inode at `/proc/sys/kernel/yama/ptrace_scope`.
struct PtraceScopeFileOps;

impl PtraceScopeFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/security/yama/yama_lsm.c#L455>
        ProcFileBuilder::new(Self, mkmod!(a+r, u+w))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for PtraceScopeFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        writeln!(printer, "1")?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, _reader: &mut VmReader) -> Result<usize> {
        warn!("writing to `/proc/sys/kernel/yama/ptrace_scope` is not supported");
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "writing to `/proc/sys/kernel/yama/ptrace_scope` is not supported"
        );
    }
}
