// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use super::{TidDirOps, process_from_pid_entry};
use crate::{
    events::IoEvents,
    fs::{
        file::{AccessMode, FileIo, StatusFlags, mkmod},
        procfs::template::{FileOpsByHandle, ProcFileBuilder},
        vfs::inode::{Inode, InodeIo},
    },
    prelude::*,
    process::{
        VmarSnapshot,
        pid_table::PidEntry,
        posix_thread::{AsPosixThread, alien_access::AlienAccessMode},
        signal::{PollHandle, Pollable},
    },
    vm::vmar::{VMAR_CAP_ADDR, VMAR_LOWEST_ADDR},
};

/// Represents the inode at `/proc/[pid]/task/[tid]/maps` (and also `/proc/[pid]/maps`).
pub struct MapsFileOps(Arc<PidEntry>);

impl MapsFileOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3343>
        ProcFileBuilder::new(Self(dir.pid_entry().clone()), mkmod!(a+r))
            .parent(parent)
            .need_revalidation()
            .build()
            .unwrap()
    }
}

impl FileOpsByHandle for MapsFileOps {
    fn open(
        &self,
        _access_mode: AccessMode,
        _status_flags: StatusFlags,
    ) -> Result<Box<dyn FileIo>> {
        let process = process_from_pid_entry(&self.0)
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "the process has been reaped"))?;
        // Hold the process VMAR lock while checking access permissions and
        // taking the VMAR identity snapshot to prevent race conditions.
        let vmar_guard = process.lock_vmar();

        process
            .main_thread()
            .as_posix_thread()
            .unwrap()
            .check_alien_access_from(
                current_thread!().as_posix_thread().unwrap(),
                AlienAccessMode::READ_WITH_FS_CREDS,
            )
            .map_err(|_| Error::with_message(Errno::EACCES, "alien access is denied"))?;

        let vmar = vmar_guard.snapshot();
        Ok(Box::new(MapsFileHandle(self.0.clone(), vmar)))
    }
}

/// A file handle opened from `/proc/[pid]/task/[tid]/maps` (and also `/proc/[pid]/maps`).
struct MapsFileHandle(Arc<PidEntry>, VmarSnapshot);

impl Pollable for MapsFileHandle {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl InodeIo for MapsFileHandle {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        let Some(process) = process_from_pid_entry(&self.0) else {
            return_errno_with_message!(Errno::ESRCH, "the process has been reaped");
        };
        let vmar_guard = process.lock_vmar();
        if !vmar_guard.is_same_as(&self.1) {
            // The process has executed a new program.
            return Ok(0);
        }
        let Some(vmar) = vmar_guard.as_ref() else {
            // The process has exited.
            return Ok(0);
        };

        let current = current_thread!();
        let fs_ref = current.as_posix_thread().unwrap().read_fs();
        let path_resolver = fs_ref.resolver().read();

        // To maintain a consistent lock order and avoid race conditions, we must lock the heap
        // before querying the VMAR.
        let heap_guard = vmar.process_vm().heap().lock();
        let guard = vmar.query(VMAR_LOWEST_ADDR..VMAR_CAP_ADDR);
        for vm_mapping in guard.iter() {
            vm_mapping.print_to_maps(&mut printer, vmar, &heap_guard, &path_resolver)?;
        }

        Ok(printer.bytes_written())
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EPERM, "`/proc/[pid]/maps` is not writable");
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
