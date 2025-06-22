// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU64, Ordering};

use super::clockid_t;
use crate::{
    events::IoEvents,
    fs::{
        file_handle::FileLike,
        utils::{CreationFlags, InodeMode, InodeType, Metadata, StatusFlags},
    },
    prelude::*,
    process::{
        signal::{PollHandle, Pollable, Pollee},
        Gid, Uid,
    },
    syscall::create_timer,
    time::{clocks::RealTimeClock, Timer},
};

/// A file-like object representing a timer that can be used with file descriptors.
pub struct TimerfdFile {
    timer: Arc<Timer>,
    ticks: Arc<AtomicU64>,
    pollee: Pollee,
    flags: SpinLock<TFDFlags>,
}

bitflags! {
    /// The flags used for timerfd-related operations.
    pub struct TFDFlags: u32 {
        const TFD_CLOEXEC = CreationFlags::O_CLOEXEC.bits();
        const TFD_NONBLOCK = StatusFlags::O_NONBLOCK.bits();
    }
}

bitflags! {
    /// The flags used for timerfd settime operations.
    pub struct TFDSetTimeFlags: u32 {
        const TFD_TIMER_ABSTIME = 0x1;
        const TFD_TIMER_CANCEL_ON_SET = 0x2;
    }
}

impl TimerfdFile {
    /// Creates a new `TimerfdFile` instance.
    pub fn new(clockid: clockid_t, flags: TFDFlags, ctx: &Context) -> Result<Self> {
        let ticks = Arc::new(AtomicU64::new(0));
        let pollee = Pollee::new();

        let timer = {
            let ticks = ticks.clone();
            let pollee = pollee.clone();

            let expired_fn = move || {
                ticks.fetch_add(1, Ordering::Release);
                pollee.notify(IoEvents::IN);
            };
            create_timer(clockid, expired_fn, ctx)
        }?;

        Ok(TimerfdFile {
            timer,
            ticks,
            pollee,
            flags: SpinLock::new(flags),
        })
    }

    /// Gets the associated timer.
    pub fn timer(&self) -> &Arc<Timer> {
        &self.timer
    }

    /// Clears the tick count.
    pub fn clear_ticks(&self) {
        self.ticks.store(0, Ordering::Release);
    }

    fn is_nonblocking(&self) -> bool {
        self.flags.lock().contains(TFDFlags::TFD_NONBLOCK)
    }

    fn try_read(&self, writer: &mut VmWriter) -> Result<()> {
        let ticks = self.ticks.fetch_and(0, Ordering::AcqRel);

        if ticks == 0 {
            return_errno_with_message!(Errno::EAGAIN, "the counter is zero");
        }

        writer.write_fallible(&mut ticks.as_bytes().into())?;

        Ok(())
    }

    fn check_io_events(&self) -> IoEvents {
        let mut events = IoEvents::empty();

        if self.ticks.load(Ordering::Acquire) != 0 {
            events |= IoEvents::IN;
        }

        events
    }
}

impl Pollable for TimerfdFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }
}

impl FileLike for TimerfdFile {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        let read_len = core::mem::size_of::<u64>();

        if writer.avail() < read_len {
            return_errno_with_message!(Errno::EINVAL, "buf len is less len u64 size");
        }

        if self.is_nonblocking() {
            self.try_read(writer)?;
        } else {
            self.wait_events(IoEvents::IN, None, || self.try_read(writer))?;
        }

        Ok(read_len)
    }

    fn write(&self, _reader: &mut VmReader) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "the file is not valid for writing");
    }

    fn status_flags(&self) -> StatusFlags {
        if self.is_nonblocking() {
            StatusFlags::O_NONBLOCK
        } else {
            StatusFlags::empty()
        }
    }

    fn set_status_flags(&self, new_flags: StatusFlags) -> Result<()> {
        let mut flags = self.flags.lock();

        if new_flags.contains(StatusFlags::O_NONBLOCK) {
            *flags |= TFDFlags::TFD_NONBLOCK;
        } else {
            *flags &= !TFDFlags::TFD_NONBLOCK;
        }

        Ok(())
    }

    fn metadata(&self) -> Metadata {
        // This is a dummy implementation.
        // TODO: Add "anonymous inode fs" and link the file to it.
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
            type_: InodeType::NamedPipe,
            mode: InodeMode::from_bits_truncate(0o200),
            nlinks: 1,
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            rdev: 0,
        }
    }
}
