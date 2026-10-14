// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
};

/// The default upper limit on the number of memory mapping areas a process may own,
/// defined by Linux as `USHRT_MAX - MAPCOUNT_ELF_CORE_MARGIN`.
///
/// Asterinas does not currently enforce this limit; the value is exposed only to
/// report the conventional Linux default to user space.
///
/// Reference:
/// <https://elixir.bootlin.com/linux/v6.16.5/source/include/linux/mm.h#L193>
const DEFAULT_MAX_MAP_COUNT: u32 = 65530;

/// Represents the inode at `/proc/sys/vm/max_map_count`.
pub struct MaxMapCountFileOps;

impl MaxMapCountFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/mm/util.c#L753>
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for MaxMapCountFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        writeln!(printer, "{}", DEFAULT_MAX_MAP_COUNT)?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, _reader: &mut VmReader) -> Result<usize> {
        warn!("writing to `/proc/sys/vm/max_map_count` is not supported");
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "writing to `/proc/sys/vm/max_map_count` is not supported"
        );
    }
}
