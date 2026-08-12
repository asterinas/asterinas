// SPDX-License-Identifier: MPL-2.0

//! The eventfd file object and its counter semantics.
//!
//! An eventfd maintains a 64-bit counter. Userspace writes add to the counter,
//! while reads either consume the whole counter or decrement it by one in
//! semaphore mode. Both operations may wait according to the file's status
//! flags.
//!
//! `EventFile` is the file-descriptor-facing object behind an eventfd.  Its
//! `KernelEventFile` component owns the counter and notification state shared
//! with in-kernel producers and consumers.

use core::fmt::Display;

use ostd::sync::LocalIrqDisabled;

use crate::{
    events::IoEvents,
    fs::{
        file::{AccessMode, CreationFlags, FileCommon, FileLike, StatusFlags, file_table::FdFlags},
        pseudofs::AnonInodeFs,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
};

bitflags! {
    /// Flags used internally when creating an eventfd file.
    pub(crate) struct EventFileFlags: u32 {
        const EFD_SEMAPHORE = 1;
        const EFD_CLOEXEC = CreationFlags::O_CLOEXEC.bits();
        const EFD_NONBLOCK = StatusFlags::O_NONBLOCK.bits();
    }
}

/// The eventfd state shared by the file-descriptor and kernel-facing APIs.
pub(crate) struct KernelEventFile {
    // Kernel event producers may signal from IRQ context, so counter updates must not sleep.
    // Reference: <https://elixir.bootlin.com/linux/v7.0/source/fs/eventfd.c#L71>.
    counter: SpinLock<u64, LocalIrqDisabled>,
    pollee: Pollee,
    is_semaphore: bool,
}

impl KernelEventFile {
    const MAX_COUNTER_VALUE: u64 = u64::MAX - 1;

    fn new(init_val: u64, is_semaphore: bool) -> Self {
        let counter = SpinLock::new(init_val);
        let pollee = Pollee::new();
        Self {
            counter,
            pollee,
            is_semaphore,
        }
    }

    /// Gets an independently owned kernel-facing state from an eventfd file.
    /// Reference: <https://elixir.bootlin.com/linux/v7.0/source/fs/eventfd.c#L366>.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(crate) fn from_file(file: &dyn FileLike) -> Result<Arc<Self>> {
        let event_file = file
            .downcast_ref::<EventFile>()
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "the file is not an event file"))?;
        Ok(event_file.kernel_event_file.clone())
    }

    /// Reports the file-facing I/O readiness of the shared eventfd state.
    ///
    /// Kernel consumers can select the readiness relevant to their operation
    /// through the poll mask. [`IoEvents::OUT`] retains its file-facing meaning:
    /// userspace can write a value of at least one without blocking. It does not
    /// constrain [`Self::signal`].
    fn check_io_events(&self) -> IoEvents {
        let counter = self.counter.lock();

        let mut events = IoEvents::empty();

        let is_readable = *counter != 0;
        if is_readable {
            events |= IoEvents::IN;
        }

        // If it is possible to write a value of at least "1" without blocking,
        // the file is writable.
        if *counter < Self::MAX_COUNTER_VALUE {
            events |= IoEvents::OUT;
        }

        if *counter == u64::MAX {
            events |= IoEvents::ERR;
        }

        events
    }

    /// Consumes and returns the counter value.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v7.0/source/fs/eventfd.c#L176>.
    pub(crate) fn consume(&self) -> Option<u64> {
        let mut counter = self.counter.lock();
        let value = if *counter == 0 {
            return None;
        } else if self.is_semaphore {
            *counter -= 1;
            1
        } else {
            let value = *counter;
            *counter = 0;
            value
        };
        drop(counter);

        // Notify outside the IRQ-disabled critical section.
        self.pollee.notify(IoEvents::OUT);
        Some(value)
    }

    /// Increments the counter by one.
    ///
    /// Unlike userspace writes, this operation may bring the counter to
    /// `u64::MAX`, which is reported through `POLLERR`.
    /// Reference: <https://elixir.bootlin.com/linux/v7.0/source/fs/eventfd.c#L46>.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(crate) fn signal(&self) {
        let mut counter = self.counter.lock();
        *counter = counter.saturating_add(1);

        let events = if *counter == u64::MAX {
            IoEvents::IN | IoEvents::ERR
        } else {
            IoEvents::IN
        };

        drop(counter);

        // FIXME: Add per-task recursion protection if eventfd observers need to signal another
        // eventfd.
        self.pollee.notify(events);
    }
}

impl Pollable for KernelEventFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }
}

pub(crate) struct EventFile {
    // `KernelEventFile` has an independent lifetime from the outer `EventFile`.
    // Kernel bindings can continue operating on this state after the outer file is dropped.
    kernel_event_file: Arc<KernelEventFile>,
    common: FileCommon,
}

impl EventFile {
    pub(crate) fn new(init_val: u64, flags: EventFileFlags) -> Self {
        let is_semaphore = flags.contains(EventFileFlags::EFD_SEMAPHORE);
        let status_flags = if flags.contains(EventFileFlags::EFD_NONBLOCK) {
            StatusFlags::O_NONBLOCK
        } else {
            StatusFlags::empty()
        };
        let pseudo_path = AnonInodeFs::new_path(|_| "anon_inode:[eventfd]".to_string());
        Self {
            kernel_event_file: Arc::new(KernelEventFile::new(init_val, is_semaphore)),
            common: FileCommon::new(pseudo_path, status_flags),
        }
    }

    fn is_nonblocking(&self) -> bool {
        self.common.is_nonblocking()
    }

    fn try_read(&self, writer: &mut VmWriter) -> Result<()> {
        let Some(value) = self.kernel_event_file.consume() else {
            return_errno_with_message!(Errno::EAGAIN, "the counter is zero");
        };

        // Reference: <https://elixir.bootlin.com/linux/v7.0/source/fs/eventfd.c#L235>.
        writer.write_fallible(&mut value.as_bytes().into())?;
        Ok(())
    }

    /// Adds a userspace-supplied value to the counter.
    ///
    /// If the new value overflows or exceeds `MAX_COUNTER_VALUE`, the counter
    /// is not modified and this method returns `Err(EAGAIN)`.
    fn add_counter_val(&self, val: u64) -> Result<()> {
        let mut counter = self.kernel_event_file.counter.lock();

        if let Some(new_value) = (*counter).checked_add(val)
            && new_value <= KernelEventFile::MAX_COUNTER_VALUE
        {
            *counter = new_value;
        } else {
            return_errno_with_message!(Errno::EAGAIN, "the new value exceeds MAX_COUNTER_VALUE");
        }

        drop(counter);

        // Notify outside the IRQ-disabled critical section.
        self.kernel_event_file.pollee.notify(IoEvents::IN);
        Ok(())
    }
}

impl Pollable for EventFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.kernel_event_file.poll(mask, poller)
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

        if supplied_value > KernelEventFile::MAX_COUNTER_VALUE {
            return_errno_with_message!(
                Errno::EINVAL,
                "the written value exceeds MAX_COUNTER_VALUE"
            );
        }

        if self.is_nonblocking() {
            self.add_counter_val(supplied_value)?;
        } else {
            self.wait_events(IoEvents::OUT, None, || self.add_counter_val(supplied_value))?;
        }

        Ok(write_len)
    }

    fn access_mode(&self) -> AccessMode {
        // Reference: <https://elixir.bootlin.com/linux/v7.0/source/fs/eventfd.c#L401>.
        AccessMode::O_RDWR
    }

    fn common(&self) -> &FileCommon {
        &self.common
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo {
            inner: Arc<EventFile>,
            fd_flags: FdFlags,
            eventfd_count: u64,
        }

        impl Display for FdInfo {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let mut flags =
                    self.inner.common.status_flags().bits() | self.inner.access_mode() as u32;
                if self.fd_flags.contains(FdFlags::CLOEXEC) {
                    flags |= CreationFlags::O_CLOEXEC.bits();
                }

                writeln!(f, "pos:\t{}", 0)?;
                writeln!(f, "flags:\t0{:o}", flags)?;
                writeln!(f, "mnt_id:\t{}", AnonInodeFs::mount_node().id())?;
                writeln!(f, "ino:\t{}", AnonInodeFs::shared_inode().ino())?;
                writeln!(f, "eventfd-count: {:16x}", self.eventfd_count)
            }
        }

        let eventfd_count = *self.kernel_event_file.counter.lock();
        Box::new(FdInfo {
            inner: self,
            fd_flags,
            eventfd_count,
        })
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::{EventFile, EventFileFlags, KernelEventFile};
    use crate::{
        events::{EpollFile, IoEvents},
        fs::file::FileLike,
        prelude::{Arc, VmWriter},
        process::signal::Pollable,
    };

    #[ktest]
    fn kernel_event_file_rejects_non_eventfd() {
        crate::time::clocks::init_for_ktest();

        let file = EpollFile::new() as Arc<dyn FileLike>;

        assert!(KernelEventFile::from_file(file.as_ref()).is_err());
    }

    #[ktest]
    fn kernel_event_file_owns_state_after_file_drop() {
        crate::time::clocks::init_for_ktest();

        let file = Arc::new(EventFile::new(0, EventFileFlags::empty())) as Arc<dyn FileLike>;
        let kernel_event_file = KernelEventFile::from_file(file.as_ref()).unwrap();
        drop(file);

        assert!(
            !kernel_event_file
                .poll(IoEvents::IN, None)
                .contains(IoEvents::IN)
        );
        kernel_event_file.signal();
        assert!(
            kernel_event_file
                .poll(IoEvents::IN, None)
                .contains(IoEvents::IN)
        );
        assert_eq!(kernel_event_file.consume(), Some(1));
        assert!(
            !kernel_event_file
                .poll(IoEvents::IN, None)
                .contains(IoEvents::IN)
        );
    }

    #[ktest]
    fn event_file_signal_consume_and_poll() {
        crate::time::clocks::init_for_ktest();

        let event_file = EventFile::new(0, EventFileFlags::empty());

        assert!(!event_file.poll(IoEvents::IN, None).contains(IoEvents::IN));
        event_file.kernel_event_file.signal();
        event_file.kernel_event_file.signal();
        assert!(event_file.poll(IoEvents::IN, None).contains(IoEvents::IN));
        assert_eq!(event_file.kernel_event_file.consume(), Some(2));
        assert_eq!(event_file.kernel_event_file.consume(), None);
        assert!(!event_file.poll(IoEvents::IN, None).contains(IoEvents::IN));
    }

    #[ktest]
    fn event_file_respects_semaphore_mode() {
        crate::time::clocks::init_for_ktest();

        let event_file = EventFile::new(0, EventFileFlags::EFD_SEMAPHORE);

        event_file.kernel_event_file.signal();
        event_file.kernel_event_file.signal();
        assert_eq!(event_file.kernel_event_file.consume(), Some(1));
        assert!(event_file.poll(IoEvents::IN, None).contains(IoEvents::IN));
        assert_eq!(event_file.kernel_event_file.consume(), Some(1));
        assert_eq!(event_file.kernel_event_file.consume(), None);
    }

    #[ktest]
    fn event_file_reports_kernel_signal_overflow() {
        crate::time::clocks::init_for_ktest();

        let event_file =
            EventFile::new(KernelEventFile::MAX_COUNTER_VALUE, EventFileFlags::empty());

        event_file.kernel_event_file.signal();
        let events = event_file.poll(IoEvents::IN | IoEvents::ERR, None);
        assert!(events.contains(IoEvents::IN));
        assert!(events.contains(IoEvents::ERR));
        assert_eq!(event_file.kernel_event_file.consume(), Some(u64::MAX));
    }

    #[ktest]
    fn event_file_read_consumes_counter() {
        crate::time::clocks::init_for_ktest();

        let event_file = EventFile::new(42, EventFileFlags::empty());

        let mut value = [0u8; size_of::<u64>()];
        let mut writer = VmWriter::from(value.as_mut_slice()).to_fallible();
        event_file.try_read(&mut writer).unwrap();
        assert_eq!(u64::from_ne_bytes(value), 42);

        assert_eq!(*event_file.kernel_event_file.counter.lock(), 0);
        assert!(
            event_file
                .kernel_event_file
                .check_io_events()
                .contains(IoEvents::OUT)
        );
    }
}
