// SPDX-License-Identifier: MPL-2.0

use aster_systree::SysObj;
use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        procfs::{
            pid::TidDirOps,
            template::{FileOps, ProcFileBuilder},
        },
        utils::{mkmod, Inode},
    },
    prelude::*,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/cgroup` (and also `/proc/[pid]/cgroup`).
pub struct CgroupFileOps(TidDirOps);

impl CgroupFileOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3379>
        ProcFileBuilder::new(Self(dir.clone()), mkmod!(a+r))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for CgroupFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        unreachable!()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let path = self
            .0
            .process_ref
            .cgroup()
            .get()
            .map(|cgroup| cgroup.path())
            .unwrap_or_else(|| "/".into());

        let mut printer = VmPrinter::new_skip(writer, offset);
        writeln!(printer, "0::{}", path)?;

        Ok(printer.bytes_written())
    }
}
