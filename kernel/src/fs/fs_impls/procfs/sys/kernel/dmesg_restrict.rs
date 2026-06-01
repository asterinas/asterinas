// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps, read_i32_from},
        vfs::inode::Inode,
    },
    prelude::*,
};

/// Represents the inode at `/proc/sys/kernel/dmesg_restrict`.
pub struct DmesgRestrictFileOps;

impl DmesgRestrictFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for DmesgRestrictFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        let dmesg_restrict = if aster_logger::klog().dmesg_restrict() {
            1
        } else {
            0
        };
        writeln!(printer, "{}", dmesg_restrict)?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        if reader.remain() == 0 {
            return Ok(0);
        }

        let (val, read_len) = read_i32_from(reader)?;
        let dmesg_restrict = match val {
            0 => false,
            1 => true,
            _ => {
                return_errno_with_message!(Errno::EINVAL, "`dmesg_restrict` must be either 0 or 1");
            }
        };

        aster_logger::klog().set_dmesg_restrict(dmesg_restrict);
        Ok(read_len)
    }
}
