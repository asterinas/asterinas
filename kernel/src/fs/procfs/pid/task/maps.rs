// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use super::TidDirOps;
use crate::{
    events::IoEvents,
    fs::{
        inode_handle::FileIo,
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{AccessMode, Inode, InodeIo, StatusFlags, mkmod},
    },
    prelude::*,
    process::{
        Process,
        posix_thread::{
            AsPosixThread,
            ptrace::{PtraceMode, check_may_access},
        },
        signal::{PollHandle, Pollable},
    },
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
    fn read_at(&self, _offset: usize, _writer: &mut VmWriter) -> Result<usize> {
        unreachable!("should read via opened `MapsFileHandle`")
    }

    fn write_at(&self, _offset: usize, _reader: &mut VmReader) -> Result<usize> {
        unreachable!("should write via opened `MapsFileHandle`")
    }

    fn open(
        &self,
        _access_mode: AccessMode,
        _status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn FileIo>>> {
        let handle = check_may_access(
            current_thread!().as_posix_thread().unwrap(),
            self.0.main_thread().as_posix_thread().unwrap(),
            PtraceMode::READ_FSCREDS,
        )
        .map(|_| Box::new(MapsFileHandle(self.0.clone())) as Box<dyn FileIo>);

        Some(handle)
    }
}

struct MapsFileHandle(Arc<Process>);

impl InodeIo for MapsFileHandle {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        let vmar_guard = self.0.lock_vmar();
        let Some(vmar) = vmar_guard.as_ref() else {
            return Ok(0);
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

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EACCES, "`/proc/[pid]/maps` is read-only");
    }
}

impl Pollable for MapsFileHandle {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileIo for MapsFileHandle {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        true
    }
}
