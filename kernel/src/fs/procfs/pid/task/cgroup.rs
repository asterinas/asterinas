// SPDX-License-Identifier: MPL-2.0

use aster_systree::SysObj;
use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        cgroupfs::CgroupSystem,
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{mkmod, Inode},
    },
    prelude::*,
    Process,
};

/// Represents the inode at `/proc/[pid]/cgroup`.
pub struct CgroupOps(Arc<Process>);

impl CgroupOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self(process_ref), mkmod!(a+r))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for CgroupOps {
    fn data(&self) -> Result<Vec<u8>> {
        unreachable!()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let path = self
            .0
            .cgroup()
            .get()
            .map(|cgroup| cgroup.path())
            .unwrap_or_else(|| CgroupSystem::singleton().path());

        let mut printer = VmPrinter::new_skip(writer, offset);
        writeln!(printer, "0::{}", path)?;

        Ok(printer.bytes_written())
    }
}
