// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use crate::{
    events::perf::policy,
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps, read_i32_from},
        vfs::inode::Inode,
    },
    prelude::*,
};

/// Represents the inode at `/proc/sys/kernel/perf_event_paranoid`.
pub(super) struct PerfEventParanoidFileOps;

impl PerfEventParanoidFileOps {
    pub(super) fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for PerfEventParanoidFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);
        writeln!(printer, "{}", policy::paranoid())?;
        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        let (level, read_bytes) = read_i32_from(reader)?;
        if reader.remain() != 0 {
            return Err(errno!(EINVAL, "expected one perf_event_paranoid value"));
        }
        policy::set_paranoid(level)?;
        Ok(read_bytes)
    }
}
