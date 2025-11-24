// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;
use core::{
    fmt::Debug,
    sync::atomic::{AtomicU8, AtomicUsize, Ordering},
    time::Duration,
};

use aster_input::{
    event_type_codes::{EventTypes, SynEvent},
    input_dev::InputEvent,
};
use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
use ostd::{
    mm::{VmReader, VmWriter},
    sync::Mutex,
    Pod,
};

use super::EvdevDevice;
use crate::{
    events::IoEvents,
    fs::{
        inode_handle::FileIo,
        utils::{InodeIo, IoctlCmd, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    util::ring_buffer::{RbConsumer, RbProducer, RingBuffer},
};

pub(super) const EVDEV_BUFFER_SIZE: usize = 64;

// Compatible with Linux's event format.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct EvdevEvent {
    pub sec: u64,
    pub usec: u64,
    pub type_: u16,
    pub code: u16,
    pub value: i32,
}

impl EvdevEvent {
    pub fn from_event_and_time(event: &InputEvent, time: Duration) -> Self {
        let (type_, code, value) = event.to_raw();
        Self {
            sec: time.as_secs(),
            usec: time.subsec_micros() as u64,
            type_,
            code,
            value,
        }
    }
}

// Reference: <https://elixir.bootlin.com/linux/v6.17.9/source/drivers/input/evdev.c#L181-L189>
#[repr(u8)]
#[derive(Debug, Clone, Copy, TryFromInt)]
enum EvdevClock {
    Realtime = 0,
    Monotonic = 1,
    Boottime = 2,
}

impl From<EvdevClock> for u8 {
    fn from(value: EvdevClock) -> Self {
        value as _
    }
}

define_atomic_version_of_integer_like_type!(EvdevClock, try_from = true, {
    #[derive(Debug)]
    struct AtomicEvdevClock(AtomicU8);
});

/// An opened file from an evdev device (`EvdevDevice`).
pub struct EvdevFile {
    /// Consumer for reading events.
    consumer: Mutex<RbConsumer<EvdevEvent>>,
    /// Clock ID for this opened evdev file.
    clock_id: AtomicEvdevClock,
    /// Number of complete event packets available (ended with SYN_REPORT).
    packet_count: AtomicUsize,
    /// Pollee for event notification.
    pollee: Pollee,
    /// Weak reference to the evdev device that owns this evdev file.
    evdev: Weak<EvdevDevice>,
}

impl Debug for EvdevFile {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("EvdevFile")
            .field("clock_id", &self.clock_id)
            .field("packet_count", &self.packet_count)
            .finish_non_exhaustive()
    }
}

impl EvdevFile {
    pub(super) fn new(
        buffer_size: usize,
        evdev: Weak<EvdevDevice>,
    ) -> (Self, RbProducer<EvdevEvent>) {
        let (producer, consumer) = RingBuffer::new(buffer_size).split();

        let evdev_file = Self {
            consumer: Mutex::new(consumer),
            clock_id: AtomicEvdevClock::new(EvdevClock::Monotonic),
            packet_count: AtomicUsize::new(0),
            pollee: Pollee::new(),
            evdev,
        };
        (evdev_file, producer)
    }

    pub(super) fn read_clock(&self) -> Duration {
        use crate::time::clocks::{BootTimeClock, MonotonicClock, RealTimeClock};

        let clock_id = self.clock_id.load(Ordering::Relaxed);
        match clock_id {
            EvdevClock::Realtime => RealTimeClock::get().read_time(),
            EvdevClock::Monotonic => MonotonicClock::get().read_time(),
            EvdevClock::Boottime => BootTimeClock::get().read_time(),
        }
    }

    /// Checks if buffer has complete event packets.
    pub fn has_complete_packets(&self) -> bool {
        self.packet_count.load(Ordering::Relaxed) > 0
    }

    /// Increments the packet count.
    pub fn increment_packet_count(&self) {
        self.packet_count.fetch_add(1, Ordering::Relaxed);
        self.pollee.notify(IoEvents::IN);
    }

    /// Decrements the packet count.
    pub fn decrement_packet_count(&self) {
        if self.packet_count.fetch_sub(1, Ordering::Relaxed) == 1 {
            self.pollee.invalidate();
        }
    }

    /// Processes events and writes them to the writer.
    ///
    /// Returns the total number of bytes written, or [`Errno::EAGAIN`] if no events are available.
    fn process_events(&self, max_events: usize, writer: &mut VmWriter) -> Result<usize> {
        const EVENT_SIZE: usize = size_of::<EvdevEvent>();

        let mut consumer = self.consumer.lock();
        let mut event_count = 0;

        for _ in 0..max_events {
            let Some(event) = consumer.pop() else {
                break;
            };

            // Update the counter since the event has been consumed.
            if is_syn_report_event(&event) || is_syn_dropped_event(&event) {
                self.decrement_packet_count();
            }

            // Write the event to the writer.
            writer.write_val(&event)?;
            event_count += 1;
        }

        if event_count == 0 {
            return_errno_with_message!(Errno::EAGAIN, "the evdev file has no events");
        }

        Ok(event_count * EVENT_SIZE)
    }

    fn check_io_events(&self) -> IoEvents {
        // TODO: Report `IoEvents::HUP` if the device has been disconnected.

        if self.has_complete_packets() {
            IoEvents::IN
        } else {
            IoEvents::empty()
        }
    }
}

/// Checks if the event is a `SYN_REPORT` event.
fn is_syn_report_event(event: &EvdevEvent) -> bool {
    event.type_ == EventTypes::SYN.as_index() && event.code == SynEvent::Report as u16
}

/// Checks if the event is a `SYN_DROPPED` event.
fn is_syn_dropped_event(event: &EvdevEvent) -> bool {
    event.type_ == EventTypes::SYN.as_index() && event.code == SynEvent::Dropped as u16
}

impl Pollable for EvdevFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }
}

impl InodeIo for EvdevFile {
    fn read_at(
        &self,
        _offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        let requested_bytes = writer.avail();
        let max_events = requested_bytes / size_of::<EvdevEvent>();

        if max_events == 0 && requested_bytes != 0 {
            return_errno_with_message!(Errno::EINVAL, "the buffer is too short");
        }

        // TODO: Return `ENODEV` if the device has been disconnected.

        let is_nonblocking = status_flags.contains(StatusFlags::O_NONBLOCK);

        // If we're in non-blocking mode, we won't bother the user space with an incomplete packet.
        // Note that this aligns to the behavior of `check_io_events`.
        if is_nonblocking && !self.has_complete_packets() {
            return_errno_with_message!(Errno::EAGAIN, "the evdev file has no packets");
        }

        // Even if `max_events` is zero, the above checks are still needed.
        if max_events == 0 {
            return Ok(0);
        }

        if is_nonblocking {
            self.process_events(max_events, writer)
        } else {
            self.wait_events(IoEvents::IN, None, || {
                self.process_events(max_events, writer)
            })
        }
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        // TODO: In Linux, writing to evdev files is permitted and will inject input events.
        return_errno_with_message!(Errno::ENOSYS, "writing to evdev files is not supported yet");
    }
}

impl FileIo for EvdevFile {
    fn check_seekable(&self) -> Result<()> {
        return_errno_with_message!(Errno::ESPIPE, "the inode is an evdev file");
    }

    fn is_offset_aware(&self) -> bool {
        false
    }

    fn ioctl(&self, _cmd: IoctlCmd, _arg: usize) -> Result<i32> {
        // TODO: Support ioctl operations for evdev files.

        // Most ioctl implementations return `ENOTTY` for invalid ioctl commands, representing "The
        // specified operation does not apply". However, according to the Linux implementation,
        // evdev files return `EINVAL` in this case.
        // Reference: <https://elixir.bootlin.com/linux/v6.17.8/source/drivers/input/evdev.c#L1251>
        return_errno_with_message!(
            Errno::EINVAL,
            "the ioctl command is not supported by evdev files"
        );
    }
}

impl Drop for EvdevFile {
    fn drop(&mut self) {
        if let Some(evdev) = self.evdev.upgrade() {
            evdev.detach_closed_files();
        }
    }
}
