// SPDX-License-Identifier: MPL-2.0

use core::{
    fmt::Display,
    sync::atomic::{AtomicBool, Ordering},
};

use crate::{
    events::IoEvents,
    fs::{
        file_handle::FileLike,
        file_table::FdFlags,
        path::RESERVED_MOUNT_ID,
        pseudofs::AnonInodeFs,
        utils::{CreationFlags, Inode, StatusFlags},
    },
    prelude::*,
    process::{
        Process,
        signal::{PollHandle, Pollable},
    },
};

pub struct PidFile {
    process: Arc<Process>,
    is_nonblocking: AtomicBool,
}

impl Debug for PidFile {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PidFile")
            .field("process", &self.process.pid())
            .field(
                "is_nonblocking",
                &self.is_nonblocking.load(Ordering::Relaxed),
            )
            .finish_non_exhaustive()
    }
}

impl PidFile {
    pub fn new(process: Arc<Process>, is_nonblocking: bool) -> Self {
        Self {
            process,
            is_nonblocking: AtomicBool::new(is_nonblocking),
        }
    }

    fn check_io_events(&self) -> IoEvents {
        // "A PID file descriptor can be monitored using poll(2), select(2),
        // and epoll(7).  When the process that it refers to terminates, these
        // interfaces indicate the file descriptor as readable."
        // Reference: <https://man7.org/linux/man-pages/man2/pidfd_open.2.html>.
        if self.process.status().is_zombie() {
            IoEvents::IN
        } else {
            IoEvents::empty()
        }
    }

    pub(super) fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Relaxed)
    }

    pub fn process(&self) -> &Arc<Process> {
        &self.process
    }
}

impl FileLike for PidFile {
    fn read(&self, _writer: &mut VmWriter) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "PID file cannot be read");
    }

    fn write(&self, _reader: &mut VmReader) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "PID file cannot be written");
    }

    fn set_status_flags(&self, new_flags: StatusFlags) -> Result<()> {
        if new_flags.contains(StatusFlags::O_NONBLOCK) {
            self.is_nonblocking.store(true, Ordering::Relaxed);
        } else {
            self.is_nonblocking.store(false, Ordering::Relaxed);
        }

        Ok(())
    }

    fn status_flags(&self) -> StatusFlags {
        if self.is_nonblocking() {
            StatusFlags::O_NONBLOCK
        } else {
            StatusFlags::empty()
        }
    }

    fn inode(&self) -> &Arc<dyn Inode> {
        AnonInodeFs::shared_inode()
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo {
            flags: u32,
            ino: u64,
            pid: u32,
        }

        impl Display for FdInfo {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                writeln!(f, "pos:\t{}", 0)?;
                writeln!(f, "flags:\t0{:o}", self.flags)?;
                // TODO: This should be the mount ID of the pseudo filesystem.
                writeln!(f, "mnt_id:\t{}", RESERVED_MOUNT_ID)?;
                writeln!(f, "ino:\t{}", self.ino)?;
                writeln!(f, "Pid:\t{}", self.pid)?;
                // TODO: Currently we do not support PID namespaces. Just print the PID once.
                writeln!(f, "NSpid:\t{}", self.pid)
            }
        }

        let mut flags = self.status_flags().bits() | self.access_mode() as u32;
        if fd_flags.contains(FdFlags::CLOEXEC) {
            flags |= CreationFlags::O_CLOEXEC.bits();
        }

        Box::new(FdInfo {
            flags,
            ino: self.inode().ino(),
            pid: self.process.pid(),
        })
    }
}

impl Pollable for PidFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.process
            .pidfile_pollee
            .poll_with(mask, poller, || self.check_io_events())
    }
}
