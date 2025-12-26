// SPDX-License-Identifier: MPL-2.0

use core::{
    fmt::Display,
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
    time::Duration,
};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;

use super::clockid_t;
use crate::{
    events::IoEvents,
    fs::{
        file_handle::FileLike,
        file_table::FdFlags,
        path::Path,
        pseudofs::AnonInodeFs,
        utils::{CreationFlags, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    syscall::create_timer,
    time::{
        Timer,
        timer::{Timeout, TimerGuard},
    },
};

/// A file-like object representing a timer that can be used with file descriptors.
pub struct TimerfdFile {
    clockid: clockid_t,
    timer: Arc<Timer>,
    ticks: Arc<AtomicU64>,
    pollee: Pollee,
    flags: AtomicTFDFlags,
    settime_flags: AtomicTFDSetTimeFlags,
    /// The pseudo path associated with this timerfd file.
    pseudo_path: Path,
}

bitflags! {
    /// The flags used for timerfd-related operations.
    pub struct TFDFlags: u32 {
        const TFD_CLOEXEC = CreationFlags::O_CLOEXEC.bits();
        const TFD_NONBLOCK = StatusFlags::O_NONBLOCK.bits();
    }
}

// Required by `define_atomic_version_of_integer_like_type`.
impl TryFrom<u32> for TFDFlags {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self> {
        Self::from_bits(value).ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid TFDFlags"))
    }
}

// Required by `define_atomic_version_of_integer_like_type`.
impl From<TFDFlags> for u32 {
    fn from(value: TFDFlags) -> Self {
        value.bits()
    }
}

define_atomic_version_of_integer_like_type!(TFDFlags, try_from = true, {
    /// An atomic version of `TFDFlags`.
    #[derive(Debug, Default)]
    struct AtomicTFDFlags(AtomicU32);
});

bitflags! {
    /// The flags used for timerfd settime operations.
    pub struct TFDSetTimeFlags: u32 {
        const TFD_TIMER_ABSTIME = 0x1;
        const TFD_TIMER_CANCEL_ON_SET = 0x2;
    }
}

// Required by `define_atomic_version_of_integer_like_type`.
impl TryFrom<u32> for TFDSetTimeFlags {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self> {
        Self::from_bits(value)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid TFDSetTimeFlags"))
    }
}

// Required by `define_atomic_version_of_integer_like_type`.
impl From<TFDSetTimeFlags> for u32 {
    fn from(value: TFDSetTimeFlags) -> Self {
        value.bits()
    }
}

define_atomic_version_of_integer_like_type!(TFDSetTimeFlags, try_from = true, {
    /// An atomic version of `TFDSetTimeFlags`.
    #[derive(Debug, Default)]
    struct AtomicTFDSetTimeFlags(AtomicU32);
});

impl TimerfdFile {
    /// Creates a new `TimerfdFile` instance.
    pub fn new(clockid: clockid_t, flags: TFDFlags, ctx: &Context) -> Result<Self> {
        let ticks = Arc::new(AtomicU64::new(0));
        let pollee = Pollee::new();

        let timer = {
            let ticks = ticks.clone();
            let pollee = pollee.clone();

            let expired_fn = move |_guard: TimerGuard| {
                ticks.fetch_add(1, Ordering::Relaxed);
                pollee.notify(IoEvents::IN);
            };
            create_timer(clockid, expired_fn, ctx)
        }?;

        let pseudo_path = AnonInodeFs::new_path(|_| "anon_inode:[timerfd]".to_string());

        Ok(TimerfdFile {
            clockid,
            timer,
            ticks,
            pollee,
            flags: AtomicTFDFlags::new(flags),
            settime_flags: AtomicTFDSetTimeFlags::default(),
            pseudo_path,
        })
    }

    // Sets the timer's timeout and interval.
    //
    // The remaining time and old interval are saved before the settings are applied, and then
    // returned afterwards.
    pub fn set_time(
        &self,
        expire_time: Duration,
        interval: Duration,
        flags: TFDSetTimeFlags,
    ) -> (Duration, Duration) {
        let mut timer_guard = self.timer.lock();

        let (old_interval, remain) = (timer_guard.interval(), timer_guard.remain());

        timer_guard.set_interval(interval);

        // Cancel the timer and clear the ticks counter.
        timer_guard.cancel();
        self.ticks.store(0, Ordering::Relaxed);

        if expire_time != Duration::ZERO {
            if flags.contains(TFDSetTimeFlags::TFD_TIMER_CANCEL_ON_SET) {
                // TODO: Currently this flag has no effect since the system time cannot be changed.
                // Once add the support for modifying the system time, the corresponding logics for
                // this flag need to be implemented.
                warn!("TFD_TIMER_CANCEL_ON_SET is not implemented yet and has no effect");
            }

            let timeout = if flags.contains(TFDSetTimeFlags::TFD_TIMER_ABSTIME) {
                Timeout::When(expire_time)
            } else {
                Timeout::After(expire_time)
            };

            timer_guard.set_timeout(timeout);
            self.settime_flags.store(flags, Ordering::Relaxed);
        }

        (old_interval, remain)
    }

    /// Gets the timer's remaining time and interval.
    pub fn get_time(&self) -> (Duration, Duration) {
        let timer_guard = self.timer.lock();
        (timer_guard.interval(), timer_guard.remain())
    }

    fn is_nonblocking(&self) -> bool {
        self.flags
            .load(Ordering::Relaxed)
            .contains(TFDFlags::TFD_NONBLOCK)
    }

    fn try_read(&self, writer: &mut VmWriter) -> Result<()> {
        let ticks = self.ticks.fetch_and(0, Ordering::Relaxed);

        if ticks == 0 {
            return_errno_with_message!(Errno::EAGAIN, "the counter is zero");
        }

        writer.write_fallible(&mut ticks.as_bytes().into())?;

        Ok(())
    }

    fn check_io_events(&self) -> IoEvents {
        let mut events = IoEvents::empty();

        if self.ticks.load(Ordering::Relaxed) != 0 {
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
        let read_len = size_of::<u64>();

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
        self.flags
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |flags| {
                if new_flags.contains(StatusFlags::O_NONBLOCK) {
                    Some(flags | TFDFlags::TFD_NONBLOCK)
                } else {
                    Some(flags & !TFDFlags::TFD_NONBLOCK)
                }
            })
            .unwrap();

        Ok(())
    }

    fn path(&self) -> &Path {
        &self.pseudo_path
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo {
            flags: u32,
            clockid: i32,
            ticks: u64,
            settime_flags: u32,
            it_value: Duration,
            it_interval: Duration,
        }

        impl Display for FdInfo {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                writeln!(f, "pos:\t{}", 0)?;
                writeln!(f, "flags:\t0{:o}", self.flags)?;
                writeln!(f, "mnt_id:\t{}", AnonInodeFs::mount_node().id())?;
                writeln!(f, "ino:\t{}", AnonInodeFs::shared_inode().ino())?;
                writeln!(f, "clockid: {}", self.clockid)?;
                writeln!(f, "ticks: {}", self.ticks)?;
                writeln!(f, "settime flags: 0{:o}", self.settime_flags)?;
                writeln!(
                    f,
                    "it_value: ({}, {})",
                    self.it_value.as_secs(),
                    self.it_value.subsec_nanos()
                )?;
                writeln!(
                    f,
                    "it_interval: ({}, {})",
                    self.it_interval.as_secs(),
                    self.it_interval.subsec_nanos()
                )
            }
        }

        let mut flags = self.status_flags().bits() | self.access_mode() as u32;
        if fd_flags.contains(FdFlags::CLOEXEC) {
            flags |= CreationFlags::O_CLOEXEC.bits();
        }

        let timer_guard = self.timer.lock();
        Box::new(FdInfo {
            flags,
            clockid: self.clockid,
            ticks: self.ticks.load(Ordering::Relaxed),
            settime_flags: self.settime_flags.load(Ordering::Relaxed).bits(),
            it_value: timer_guard.expired_time(),
            it_interval: timer_guard.interval(),
        })
    }
}
