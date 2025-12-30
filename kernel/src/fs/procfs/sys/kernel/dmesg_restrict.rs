// SPDX-License-Identifier: MPL-2.0

use aster_logger::{dmesg_restrict_get, dmesg_restrict_set};
use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{Inode, mkmod},
    },
    prelude::*,
};

/// Represents the inode at `/proc/sys/kernel/dmesg_restrict`.
pub struct DmesgRestrictFileOps;

impl DmesgRestrictFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self, mkmod!(a+r, u+w))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for DmesgRestrictFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);
        writeln!(printer, "{}", if dmesg_restrict_get() { 1 } else { 0 })?;
        Ok(printer.bytes_written())
    }

    fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        if offset != 0 {
            return_errno_with_message!(Errno::EINVAL, "writes must start at offset 0");
        }

        let input_len = reader.remain();
        if input_len == 0 {
            return Ok(0);
        }
        if input_len > DMESG_RESTRICT_MAX_INPUT {
            return_errno_with_message!(Errno::EINVAL, "input is too long");
        }

        let mut buf = [0u8; DMESG_RESTRICT_MAX_INPUT];
        let mut writer = VmWriter::from(&mut buf[..input_len]).to_fallible();
        let read_len = match reader.read_fallible(&mut writer) {
            Ok(len) => len,
            Err((err, 0)) => return Err(err.into()),
            Err((err, _written)) => return Err(err.into()),
        };

        let input = core::str::from_utf8(&buf[..read_len])
            .map_err(|_| Error::new(Errno::EINVAL))?
            .trim();

        let val = match input {
            "0" => false,
            "1" => true,
            _ => return_errno_with_message!(Errno::EINVAL, "expected 0 or 1"),
        };

        dmesg_restrict_set(val);
        Ok(read_len)
    }
}

const DMESG_RESTRICT_MAX_INPUT: usize = 16;
