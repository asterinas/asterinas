// SPDX-License-Identifier: MPL-2.0

//! `eventfd()` creates an "eventfd object" (we name it as `EventFile`)
//! which serves as a mechanism for event wait/notify.
//!
//! `EventFile` holds a u64 integer counter.
//! Writing to `EventFile` increments the counter by the written value.
//! Reading from `EventFile` returns the current counter value and resets it
//! (It is also possible to only read 1,
//! depending on whether the `EFD_SEMAPHORE` flag is set).
//! The read/write operations may be blocked based on file flags.
//!
//! For more detailed information about this syscall,
//! refer to the man 2 eventfd documentation.
//!

use ostd::sync::WaitQueue;

use super::SyscallReturn;
use crate::{
    events::IoEvents,
    fs::{
        file_handle::FileLike,
        file_table::{FdFlags, FileDesc},
        utils::{CreationFlags, InodeMode, InodeType, Metadata, StatusFlags},
    },
    prelude::*,
    process::{
        signal::{PollHandle, Pollable, Pollee},
        Gid, Uid,
    },
    time::clocks::RealTimeClock,
};

pub fn sys_eventfd(init_val: u64, ctx: &Context) -> Result<SyscallReturn> {
    debug!("init_val = 0x{:x}", init_val);

    let fd = do_sys_eventfd2(init_val, Flags::empty(), ctx);

    Ok(SyscallReturn::Return(fd as _))
}

pub fn sys_eventfd2(init_val: u64, flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    trace!("raw flags = {}", flags);
    let flags = Flags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown flags"))?;
    debug!("init_val = 0x{:x}, flags = {:?}", init_val, flags);

    let fd = do_sys_eventfd2(init_val, flags, ctx);

    Ok(SyscallReturn::Return(fd as _))
}

fn do_sys_eventfd2(init_val: u64, flags: Flags, ctx: &Context) -> FileDesc {
    let event_file = EventFile::new(init_val, flags);
    let fd = {
        let mut file_table = ctx.process.file_table().lock();
        let fd_flags = if flags.contains(Flags::EFD_CLOEXEC) {
            FdFlags::CLOEXEC
        } else {
            FdFlags::empty()
        };
        file_table.insert(Arc::new(event_file), fd_flags)
    };
    fd
}

bitflags! {
    struct Flags: u32 {
        const EFD_SEMAPHORE = 1;
        const EFD_CLOEXEC = CreationFlags::O_CLOEXEC.bits();
        const EFD_NONBLOCK = StatusFlags::O_NONBLOCK.bits();
    }
}

struct EventFile {
    counter: Mutex<u64>,
    pollee: Pollee,
    flags: Mutex<Flags>,
    write_wait_queue: WaitQueue,
}

impl EventFile {
    const MAX_COUNTER_VALUE: u64 = u64::MAX - 1;

    fn new(init_val: u64, flags: Flags) -> Self {
        let counter = Mutex::new(init_val);
        let pollee = Pollee::new();
        let write_wait_queue = WaitQueue::new();
        Self {
            counter,
            pollee,
            flags: Mutex::new(flags),
            write_wait_queue,
        }
    }

    fn is_nonblocking(&self) -> bool {
        self.flags.lock().contains(Flags::EFD_NONBLOCK)
    }

    fn check_io_events(&self) -> IoEvents {
        let counter = self.counter.lock();

        let mut events = IoEvents::empty();

        let is_readable = *counter != 0;
        if is_readable {
            events |= IoEvents::IN;
        }

        // if it is possible to write a value of at least "1"
        // without blocking, the file is writable
        let is_writable = *counter < Self::MAX_COUNTER_VALUE;
        if is_writable {
            events |= IoEvents::OUT;
        }

        events
    }

    fn try_read(&self, writer: &mut VmWriter) -> Result<()> {
        let mut counter = self.counter.lock();

        // Wait until the counter becomes non-zero
        if *counter == 0 {
            return_errno_with_message!(Errno::EAGAIN, "the counter is zero");
        }

        // Copy value from counter, and set the new counter value
        if self.flags.lock().contains(Flags::EFD_SEMAPHORE) {
            writer.write_fallible(&mut 1u64.as_bytes().into())?;
            *counter -= 1;
        } else {
            writer.write_fallible(&mut (*counter).as_bytes().into())?;
            *counter = 0;
        }

        self.pollee.notify(IoEvents::OUT);
        self.write_wait_queue.wake_all();

        Ok(())
    }

    /// Adds val to the counter.
    ///
    /// If the new_value is overflowed or exceeds MAX_COUNTER_VALUE, the counter value
    /// will not be modified, and this method returns `Err(EINVAL)`.
    fn add_counter_val(&self, val: u64) -> Result<()> {
        let mut counter = self.counter.lock();

        let new_value = (*counter)
            .checked_add(val)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "arithmetic overflow"))?;

        if new_value <= Self::MAX_COUNTER_VALUE {
            *counter = new_value;
            self.pollee.notify(IoEvents::IN);
            return Ok(());
        }

        return_errno_with_message!(Errno::EINVAL, "new value exceeds MAX_COUNTER_VALUE");
    }
}

impl Pollable for EventFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }
}

impl FileLike for EventFile {
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

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        let write_len = core::mem::size_of::<u64>();
        if reader.remain() < write_len {
            return_errno_with_message!(Errno::EINVAL, "buf len is less than the size of u64");
        }

        let supplied_value = reader.read_val::<u64>()?;

        // Try to add counter val at first
        if self.add_counter_val(supplied_value).is_ok() {
            return Ok(write_len);
        }

        if self.is_nonblocking() {
            return_errno_with_message!(Errno::EAGAIN, "try writing to event file again");
        }

        // Wait until counter can be added val to
        self.write_wait_queue
            .pause_until(|| self.add_counter_val(supplied_value).ok())?;

        Ok(write_len)
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
            *flags |= Flags::EFD_NONBLOCK;
        } else {
            *flags &= !Flags::EFD_NONBLOCK;
        }

        // TODO: deal with other flags

        Ok(())
    }

    fn metadata(&self) -> Metadata {
        // This is a dummy implementation.
        // TODO: Add "anonymous inode fs" and link `EventFile` to it.
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
