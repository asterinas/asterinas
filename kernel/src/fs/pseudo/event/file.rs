// SPDX-License-Identifier: MPL-2.0

use ostd::sync::WaitQueue;

use crate::{
    events::IoEvents,
    fs::{
        file_handle::FileLike,
        path::Dentry,
        pseudo::alloc_anon_dentry,
        utils::{CreationFlags, Metadata, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
};

pub struct EventFile {
    counter: Mutex<u64>,
    pollee: Pollee,
    flags: Mutex<Flags>,
    write_wait_queue: WaitQueue,
    dentry: Dentry,
}

impl EventFile {
    const MAX_COUNTER_VALUE: u64 = u64::MAX - 1;

    pub fn new(init_val: u64, flags: Flags) -> Self {
        let dentry = alloc_anon_dentry("[eventfd]").unwrap();
        let counter = Mutex::new(init_val);
        let pollee = Pollee::new();
        let write_wait_queue = WaitQueue::new();
        Self {
            counter,
            pollee,
            flags: Mutex::new(flags),
            write_wait_queue,
            dentry,
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

    pub fn dentry(&self) -> &Dentry {
        &self.dentry
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
        self.dentry().metadata()
    }
}

bitflags! {
    pub struct Flags: u32 {
        const EFD_SEMAPHORE = 1;
        const EFD_CLOEXEC = CreationFlags::O_CLOEXEC.bits();
        const EFD_NONBLOCK = StatusFlags::O_NONBLOCK.bits();
    }
}
