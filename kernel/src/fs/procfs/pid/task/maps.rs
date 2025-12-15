// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use super::TidDirOps;
use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{Inode, mkmod},
    },
    prelude::*,
    process::Process,
    vm::vmar::{VMAR_CAP_ADDR, VMAR_LOWEST_ADDR},
};

/// Represents the inode at `/proc/[pid]/task/[tid]/maps` (and also `/proc/[pid]/maps`).
pub struct MapsFileOps(Arc<Process>);

impl MapsFileOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        let process_ref = dir.process_ref.clone();
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3343>
        ProcFileBuilder::new(Self(process_ref), mkmod!(a+r))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for MapsFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        let vmar_guard = self.0.lock_vmar();
        let Some(vmar) = vmar_guard.as_ref() else {
            return_errno_with_message!(Errno::ESRCH, "the process has exited");
        };

        let user_stack_top = vmar.process_vm().init_stack().user_stack_top();

        let guard = vmar.query(VMAR_LOWEST_ADDR..VMAR_CAP_ADDR);
        for vm_mapping in guard.iter() {
            if vm_mapping.map_to_addr() <= user_stack_top && vm_mapping.map_end() > user_stack_top {
                vm_mapping.print_to_maps(&mut printer, "[stack]")?;
            } else {
                // TODO: Print the status of mappings other than the stack.
                continue;
            }
        }

        Ok(printer.bytes_written())
    }
}
