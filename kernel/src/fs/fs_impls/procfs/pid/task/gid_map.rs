// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use super::TidDirOps;
use crate::{
    fs::{
        file::mkmod,
        procfs::template::{FileOps, ProcFileBuilder},
        vfs::inode::Inode,
    },
    prelude::*,
    process::{Gid, Process},
};

/// Represents the inode at `/proc/[pid]/task/[tid]/gid_map` (and also `/proc/[pid]/gid_map`).
#[expect(dead_code)]
pub struct GidMapFileOps(Arc<Process>);

impl GidMapFileOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        let process_ref = dir.process_ref.clone();
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3403>
        ProcFileBuilder::new(Self(process_ref), mkmod!(a+r, u+w))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for GidMapFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        // This is the default GID map for the initial user namespace.
        // TODO: Retrieve the GID map from the user namespace of the current process
        // instead of returning this hard-coded value.
        writeln!(
            printer,
            "{:>10} {:>10} {:>10}",
            0,
            0,
            u32::from(Gid::INVALID)
        )?;

        Ok(printer.bytes_written())
    }
}
