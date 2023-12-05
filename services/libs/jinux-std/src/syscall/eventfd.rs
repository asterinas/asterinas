//! `eventfd()` creates an "eventfd object" (we name it as `EventFile`) which serves as a
//! mechanism for event wait/notify.
//!
//! `EventFile` holds a u64 integer counter. Writing to `EventFile` increments the counter
//! by the written value. Reading from EventFile returns the current counter value and
//! resets it (It is also possible to only read 1, depending on whether the `EFD_SEMAPHORE`
//! flag is set.). The read/write operations may be blocked based on certain requirements.
//!
//! For more detailed information about this syscall, refer to the man 2 eventfd documentation.

use super::{SyscallReturn, SYS_EVENTFD, SYS_EVENTFD2};
use crate::events::{IoEvents, Observer};
use crate::fs::file_handle::FileLike;
use crate::fs::utils::{CreationFlags, StatusFlags};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::signal::{Pauser, Pollee, Poller};

pub fn sys_eventfd(init_val: u64) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_EVENTFD);
    debug!("init_val = 0x{:x}", init_val);

    let event_file = {
        let flags = Flags::empty();
        EventFile::new(init_val, flags)
    };

    let fd = {
        let current = current!();
        let mut file_table = current.file_table().lock();
        // TODO: deal with close_on_exec
        file_table.insert(Arc::new(event_file))
    };

    Ok(SyscallReturn::Return(fd as _))
}

pub fn sys_eventfd2(init_val: u64, flags: u32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_EVENTFD2);
    trace!("raw flags = {}", flags);
    let flags = Flags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown flags"))?;
    debug!("init_val = 0x{:x}, flags = {:?}", init_val, flags);

    let event_file = EventFile::new(init_val, flags);
    let fd = {
        let current = current!();
        let mut file_table = current.file_table().lock();
        // TODO: deal with close_on_exec
        file_table.insert(Arc::new(event_file))
    };

    Ok(SyscallReturn::Return(fd as _))
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
    writing_pauser: Arc<Pauser>,
}

impl EventFile {
    const MAX_COUNTER_VALUE: u64 = u64::MAX - 1;

    fn new(init_val: u64, flags: Flags) -> Self {
        let counter = Mutex::new(init_val);
        let pollee = Pollee::new(IoEvents::OUT);
        let writing_pauser = Pauser::new();
        Self {
            counter,
            pollee,
            flags: Mutex::new(flags),
            writing_pauser,
        }
    }

    fn is_nonblocking(&self) -> bool {
        self.flags.lock().contains(Flags::EFD_NONBLOCK)
    }

    fn update_io_state(&self, counter: &MutexGuard<u64>) {
        if **counter == 0 {
            self.pollee.del_events(IoEvents::IN);
        } else {
            self.pollee.add_events(IoEvents::IN);
        }

        // if it is possible to write a value of at least "1" without
        // blocking, the file is writable
        if **counter < Self::MAX_COUNTER_VALUE {
            self.pollee.add_events(IoEvents::OUT);
            self.writing_pauser.resume_all();
        } else {
            self.pollee.del_events(IoEvents::OUT);
        }

        // TODO: deal with overflow logic
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
            self.update_io_state(&counter);
            return Ok(());
        }

        return_errno_with_message!(Errno::EINVAL, "new value exceeds MAX_COUNTER_VALUE");
    }
}

impl FileLike for EventFile {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        let read_len = core::mem::size_of::<u64>();
        if buf.len() < read_len {
            return_errno_with_message!(Errno::EINVAL, "buf len is less len u64 size");
        }

        loop {
            let mut counter = self.counter.lock();

            // Wait until the counter becomes non-zero
            if *counter == 0 {
                if self.is_nonblocking() {
                    return_errno_with_message!(Errno::EAGAIN, "try reading event file again");
                }

                self.update_io_state(&counter);
                drop(counter);

                let poller = Poller::new();
                if self.pollee.poll(IoEvents::IN, Some(&poller)).is_empty() {
                    poller.wait()?;
                }
                continue;
            }

            // Copy value from counter, and set the new counter value
            if self.flags.lock().contains(Flags::EFD_SEMAPHORE) {
                buf[..read_len].copy_from_slice(1u64.as_bytes());
                *counter -= 1;
            } else {
                buf[..read_len].copy_from_slice((*counter).as_bytes());
                *counter = 0;
            }

            self.update_io_state(&counter);
            break;
        }

        Ok(read_len)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        let write_len = core::mem::size_of::<u64>();
        if buf.len() < write_len {
            return_errno_with_message!(Errno::EINVAL, "buf len is less than u64 size");
        }

        let supplied_value = u64::from_bytes(buf);

        // Try to add counter val at first
        if self.add_counter_val(supplied_value).is_ok() {
            return Ok(write_len);
        }

        if self.is_nonblocking() {
            return_errno_with_message!(Errno::EAGAIN, "try writing to event file again");
        }

        // Wait until counter can be added val to
        self.writing_pauser
            .pause_until(|| self.add_counter_val(supplied_value).ok())?;

        Ok(write_len)
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
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

        Ok(())
    }

    fn register_observer(
        &self,
        observer: Weak<dyn crate::events::Observer<IoEvents>>,
        mask: IoEvents,
    ) -> Result<()> {
        self.pollee.register_observer(observer, mask);
        Ok(())
    }

    fn unregister_observer(
        &self,
        observer: &Weak<dyn Observer<IoEvents>>,
    ) -> Result<Weak<dyn Observer<IoEvents>>> {
        self.pollee
            .unregister_observer(observer)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "cannot unregister observer"))
    }
}
