// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use super::TidDirOps;
use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{Inode, mkmod},
    },
    prelude::*,
    process::{Process, posix_thread::AsPosixThread},
    vm::vmar::{VmMapping, userspace_range},
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

        let current = current_thread!();
        let fs_ref = current.as_posix_thread().unwrap().read_fs();
        let path_resolver = fs_ref.resolver().read();

        let mut mappings: Vec<VmMapping> = Vec::new();

        let _ = vmar.for_each_mapping(userspace_range(), false, |vm_mapping| {
            if let Some(last) = mappings.last_mut()
                && last.can_merge_with(vm_mapping)
            {
                let merged = mappings.pop().unwrap().try_merge_with(vm_mapping).0;
                mappings.push(merged);
            } else {
                mappings.push(vm_mapping.clone_for_check());
            }
        });

        for mapping in mappings {
            mapping.print_to_maps(&mut printer, vmar, &path_resolver)?;
        }

        Ok(printer.bytes_written())
    }
}
