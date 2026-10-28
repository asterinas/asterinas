// SPDX-License-Identifier: MPL-2.0

//! The notification listener's file lifetime and attached-thread accounting.

use core::{
    fmt::{self, Display},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use crate::{
    events::IoEvents,
    fs::{
        file::{AccessMode, CreationFlags, FileCommon, FileLike, StatusFlags, file_table::FdFlags},
        pseudofs::AnonInodeFs,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    util::ioctl::RawIoctl,
};

/// Notification state shared by a filter and its listener file description.
pub(super) struct Listener {
    is_open: AtomicBool,
    users: AtomicUsize,
    pollee: Pollee,
}

impl Listener {
    pub(super) fn new() -> Result<Arc<Self>> {
        Arc::try_new(Self {
            is_open: AtomicBool::new(true),
            users: AtomicUsize::new(0),
            pollee: Pollee::new(),
        })
        .map_err(|_| Error::with_message(Errno::ENOMEM, "cannot allocate a seccomp listener"))
    }

    pub(super) fn is_open(&self) -> bool {
        self.is_open.load(Ordering::Acquire)
    }

    pub(super) fn attach_thread(&self) {
        // Only attached threads count; temporary policy snapshots and listener
        // files cannot prevent POLLHUP. The count is bounded by the TID space.
        self.users.fetch_add(1, Ordering::Relaxed);
        self.pollee.invalidate();
    }

    pub(super) fn detach_thread(&self) {
        let previous = self.users.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0);
        if previous == 1 {
            self.pollee.notify(IoEvents::HUP);
        }
    }

    fn check_events(&self) -> IoEvents {
        if self.users.load(Ordering::Acquire) == 0 {
            IoEvents::HUP
        } else {
            IoEvents::empty()
        }
    }
}

/// An open seccomp notification listener, shared by dup and descriptor passing.
pub(super) struct ListenerFile {
    listener: Arc<Listener>,
    common: FileCommon,
}

impl ListenerFile {
    pub(super) fn new(listener: Arc<Listener>) -> Result<Arc<Self>> {
        let path = AnonInodeFs::new_path(|_| "anon_inode:seccomp notify".to_string());
        Arc::try_new(Self {
            listener,
            common: FileCommon::new(path, AccessMode::O_RDWR, StatusFlags::empty()),
        })
        .map_err(|_| Error::with_message(Errno::ENOMEM, "cannot allocate a seccomp listener file"))
    }
}

impl Drop for ListenerFile {
    fn drop(&mut self) {
        self.listener.is_open.store(false, Ordering::Release);
    }
}

impl Pollable for ListenerFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.listener.poll(mask, poller)
    }
}

impl Pollable for Listener {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee.poll_with(mask, poller, || self.check_events())
    }
}

impl FileLike for ListenerFile {
    fn ioctl(&self, _raw_ioctl: RawIoctl) -> Result<i32> {
        // Notification protocol operations are introduced separately from the
        // listener installation transaction. Linux rejects unknown ioctls with EINVAL.
        return_errno_with_message!(Errno::EINVAL, "unsupported seccomp notification ioctl")
    }

    fn common(&self) -> &FileCommon {
        &self.common
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo {
            flags: u32,
        }
        impl Display for FdInfo {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                writeln!(f, "pos:\t0")?;
                writeln!(f, "flags:\t0{:o}", self.flags)?;
                writeln!(f, "mnt_id:\t{}", AnonInodeFs::mount_node().id())?;
                writeln!(f, "ino:\t{}", AnonInodeFs::shared_inode().ino())
            }
        }
        let mut flags = self.common.status_flags().bits() | self.common.access_mode() as u32;
        if fd_flags.contains(FdFlags::CLOEXEC) {
            flags |= CreationFlags::O_CLOEXEC.bits();
        }
        Box::new(FdInfo { flags })
    }
}
