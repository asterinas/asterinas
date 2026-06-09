// SPDX-License-Identifier: MPL-2.0

//! `eventfd()` creates an "eventfd object" (we name it as `EventFile`)
//! which serves as a mechanism for event wait/notify.
//!
//! `EventFile` holds a `u64` integer counter.
//! Writing to `EventFile` increments the counter by the written value.
//! Reading from `EventFile` returns the current counter value and resets it
//! (It is also possible to only read 1,
//! depending on whether the `EFD_SEMAPHORE` flag is set).
//! The read/write operations may be blocked based on file flags.
//!
//! For more detailed information about this syscall,
//! refer to the man 2 eventfd documentation.
//!

use core::fmt::Display;

use ostd::sync::WaitQueue;

use super::SyscallReturn;
use crate::{
    events::IoEvents,
    fs::{
        file::{AccessMode, CreationFlags, FileLike, StatusFlags, file_table::FdFlags},
        pseudofs::AnonInodeFs,
        vfs::path::Path,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
};

pub fn sys_eventfd(init_val: u32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("init_val = 0x{:x}", init_val);

    do_sys_eventfd2(init_val, Flags::empty(), ctx)
}

pub fn sys_eventfd2(init_val: u32, flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("raw flags = {}", flags);
    let flags = Flags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown flags"))?;
    debug!("init_val = 0x{:x}, flags = {:?}", init_val, flags);

    do_sys_eventfd2(init_val, flags, ctx)
}

fn do_sys_eventfd2(init_val: u32, flags: Flags, ctx: &Context) -> Result<SyscallReturn> {
    let event_file = EventFile::new(init_val as u64, flags);
    let file_table = ctx.thread_local.borrow_file_table();
    let mut file_table_locked = file_table.unwrap().write();
    let fd_flags = if flags.contains(Flags::EFD_CLOEXEC) {
        FdFlags::CLOEXEC
    } else {
        FdFlags::empty()
    };

    let fd = file_table_locked.insert(Arc::new(event_file), fd_flags);
    Ok(SyscallReturn::Return(fd.into()))
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
    /// The pseudo path associated with this eventfd file.
    pseudo_path: Path,
}

impl EventFile {
    const MAX_COUNTER_VALUE: u64 = u64::MAX - 1;

    fn new(init_val: u64, flags: Flags) -> Self {
        let counter = Mutex::new(init_val);
        let pollee = Pollee::new();
        let write_wait_queue = WaitQueue::new();
        let pseudo_path = AnonInodeFs::new_path(|_| "anon_inode:[eventfd]".to_string());
        Self {
            counter,
            pollee,
            flags: Mutex::new(flags),
            write_wait_queue,
            pseudo_path,
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

        // If it is possible to write a value of at least "1"
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

        // Copy the value from the counter and set the new counter value
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

    /// Adds a value to the counter.
    ///
    /// If the new value overflows or exceeds `MAX_COUNTER_VALUE`, the counter value
    /// will not be modified and this method will return `Err(EAGAIN)`.
    fn add_counter_val(&self, val: u64) -> Result<()> {
        let mut counter = self.counter.lock();

        if let Some(new_value) = (*counter).checked_add(val)
            && new_value <= Self::MAX_COUNTER_VALUE
        {
            *counter = new_value;
            self.pollee.notify(IoEvents::IN);
            return Ok(());
        }

        return_errno_with_message!(Errno::EAGAIN, "the new value exceeds MAX_COUNTER_VALUE");
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
        let read_len = size_of::<u64>();
        if writer.avail() < read_len {
            return_errno_with_message!(Errno::EINVAL, "the event buffer is too small");
        }

        if self.is_nonblocking() {
            self.try_read(writer)?;
        } else {
            self.wait_events(IoEvents::IN, None, || self.try_read(writer))?;
        }

        Ok(read_len)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        let write_len = size_of::<u64>();
        if reader.remain() < write_len {
            return_errno_with_message!(Errno::EINVAL, "the event buffer is too small");
        }

        let supplied_value = reader.read_val::<u64>()?;

        if supplied_value > EventFile::MAX_COUNTER_VALUE {
            return_errno_with_message!(
                Errno::EINVAL,
                "the written value exceeds MAX_COUNTER_VALUE"
            );
        }

        if self.is_nonblocking() {
            // Try to add the value
            self.add_counter_val(supplied_value)?;
        } else {
            // Wait until the value can be added
            self.write_wait_queue
                .pause_until(|| self.add_counter_val(supplied_value).ok())?;
        }

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

        // TODO: Deal with other flags

        Ok(())
    }

    fn access_mode(&self) -> AccessMode {
        // Reference: <https://elixir.bootlin.com/linux/v7.0/source/fs/eventfd.c#L401>.
        AccessMode::O_RDWR
    }

    fn path(&self) -> &Path {
        &self.pseudo_path
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo {
            inner: Arc<EventFile>,
            fd_flags: FdFlags,
        }

        impl Display for FdInfo {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let mut flags = self.inner.status_flags().bits() | self.inner.access_mode() as u32;
                if self.fd_flags.contains(FdFlags::CLOEXEC) {
                    flags |= CreationFlags::O_CLOEXEC.bits();
                }

                writeln!(f, "pos:\t{}", 0)?;
                writeln!(f, "flags:\t0{:o}", flags)?;
                writeln!(f, "mnt_id:\t{}", AnonInodeFs::mount_node().id())?;
                writeln!(f, "ino:\t{}", AnonInodeFs::shared_inode().ino())?;
                writeln!(f, "eventfd-count: {:16x}", *self.inner.counter.lock())
            }
        }

        Box::new(FdInfo {
            inner: self,
            fd_flags,
        })
    }
}
