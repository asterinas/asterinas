// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use crate::{
    events::IoEvents,
    fs::{
        file_handle::FileLike,
        utils::{InodeMode, InodeType, Metadata, StatusFlags},
    },
    prelude::*,
    process::{
        signal::{PollHandle, Pollable, Pollee},
        Gid, Process, Uid,
    },
    time::clocks::RealTimeClock,
};

pub struct PidFile {
    process: Arc<Process>,
    is_nonblocking: AtomicBool,
    pollee: Pollee,
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
        let pollee = process.pidfile_pollee.clone();
        Self {
            process,
            is_nonblocking: AtomicBool::new(is_nonblocking),
            pollee,
        }
    }

    fn check_io_events(&self) -> IoEvents {
        if self.process.status().is_zombie() {
            IoEvents::IN
        } else {
            IoEvents::empty()
        }
    }

    pub(super) fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Relaxed)
    }

    pub(super) fn process(&self) -> &Arc<Process> {
        &self.process
    }
}

impl FileLike for PidFile {
    fn metadata(&self) -> Metadata {
        let now = RealTimeClock::get().read_time();
        Metadata {
            dev: 0,
            ino: 0,
            size: 0,
            blk_size: 0,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            type_: InodeType::Unknown,
            mode: InodeMode::from_bits_truncate(0o600),
            nlinks: 1,
            // FIXME: Should we use the process's uid here?
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            rdev: 0,
        }
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
}

impl Pollable for PidFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }
}
