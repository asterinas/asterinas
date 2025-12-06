// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;
use core::{
    cmp,
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
    current_userspace,
    events::IoEvents,
    fs::{
        inode_handle::FileIo,
        utils::{InodeIo, IoctlCmd, IoctlDir, IoctlRequest, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    syscall::ClockId,
    util::ring_buffer::{RbConsumer, RbProducer, RingBuffer},
};

pub(super) const EVDEV_BUFFER_SIZE: usize = 64;

const EVDEV_IOCTL_TYPE: u8 = b'E';

crate::define_ioctl_cmd!(
    GetVersion,
    EVDEV_IOCTL_TYPE,
    0x01,
    crate::fs::utils::IoctlDir::Read,
    i32
);

crate::define_ioctl_cmd!(
    GetId,
    EVDEV_IOCTL_TYPE,
    0x02,
    crate::fs::utils::IoctlDir::Read,
    aster_input::input_dev::InputId
);

crate::define_ioctl_cmd!(
    SetClockId,
    EVDEV_IOCTL_TYPE,
    0xa0,
    crate::fs::utils::IoctlDir::Write,
    i32
);

crate::define_ioctl_cmd!(
    GetDeviceName,
    EVDEV_IOCTL_TYPE,
    0x06,
    crate::fs::utils::IoctlDir::Read,
    [u8]
);

crate::define_ioctl_cmd!(
    GetPhys,
    EVDEV_IOCTL_TYPE,
    0x07,
    crate::fs::utils::IoctlDir::Read,
    [u8]
);

crate::define_ioctl_cmd!(
    GetUniq,
    EVDEV_IOCTL_TYPE,
    0x08,
    crate::fs::utils::IoctlDir::Read,
    [u8]
);

crate::define_ioctl_cmd!(
    GetKeyState,
    EVDEV_IOCTL_TYPE,
    0x18,
    crate::fs::utils::IoctlDir::Read,
    [u8]
);

crate::define_ioctl_cmd!(
    GetLedState,
    EVDEV_IOCTL_TYPE,
    0x19,
    crate::fs::utils::IoctlDir::Read,
    [u8]
);

crate::define_ioctl_cmd!(
    GetSwState,
    EVDEV_IOCTL_TYPE,
    0x1B,
    crate::fs::utils::IoctlDir::Read,
    [u8]
);

struct GetBit {
    event_type: u16,
    request: IoctlRequest,
}

impl GetBit {
    const BASE_NR: u8 = 0x20;

    fn event_type(&self) -> u16 {
        self.event_type
    }

    fn buffer_len(&self) -> usize {
        self.request.buffer_len()
    }

    fn user_ptr(&self) -> usize {
        self.request.user_ptr()
    }
}

impl TryFrom<IoctlRequest> for GetBit {
    type Error = Error;

    fn try_from(request: IoctlRequest) -> Result<Self> {
        if request.direction() != IoctlDir::Read
            || request.type_id() != EVDEV_IOCTL_TYPE
            || request.number() < Self::BASE_NR
        {
            return Err(Error::with_message(Errno::EINVAL, "ioctl command mismatch"));
        }
        Ok(Self {
            event_type: (request.number() - Self::BASE_NR) as u16,
            request,
        })
    }
}

// Reference: <https://elixir.bootlin.com/linux/v6.17.9/source/include/uapi/linux/input.h#L28>
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub(super) struct EvdevEvent {
    sec: u64,
    usec: u64,
    type_: u16,
    code: u16,
    value: i32,
}

impl EvdevEvent {
    pub(super) fn from_event_and_time(event: &InputEvent, time: Duration) -> Self {
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

/// An opened file from an evdev device ([`EvdevDevice`]).
pub(super) struct EvdevFile {
    /// Inner data (shared with the device).
    inner: Arc<EvdevFileInner>,
    /// Weak reference to the evdev device that owns this evdev file.
    evdev: Weak<EvdevDevice>,
}

/// An opened evdev file's inner data (shared with its [`EvdevDevice`]).
pub(super) struct EvdevFileInner {
    /// Consumer for reading events.
    consumer: Mutex<RbConsumer<EvdevEvent>>,
    /// Clock ID for this opened evdev file.
    clock_id: AtomicEvdevClock,
    /// Number of complete event packets available (ended with `SYN_REPORT`).
    packet_count: AtomicUsize,
    /// Pollee for event notification.
    pollee: Pollee,
}

impl Debug for EvdevFile {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("EvdevFile")
            .field("clock_id", &self.inner.clock_id)
            .field("packet_count", &self.inner.packet_count)
            .finish_non_exhaustive()
    }
}

impl EvdevFileInner {
    pub(super) fn read_clock(&self) -> Duration {
        use crate::time::clocks::{BootTimeClock, MonotonicClock, RealTimeClock};

        let clock_id = self.clock_id.load(Ordering::Relaxed);
        match clock_id {
            EvdevClock::Realtime => RealTimeClock::get().read_time(),
            EvdevClock::Monotonic => MonotonicClock::get().read_time(),
            EvdevClock::Boottime => BootTimeClock::get().read_time(),
        }
    }

    pub(super) fn try_clear_with_producer_locked(&self) {
        let Some(mut consumer) = self.consumer.try_lock() else {
            return;
        };

        // Note that the following two operations are racy unless we hold the producer's lock.
        consumer.clear();
        self.packet_count.store(0, Ordering::Relaxed);
        self.pollee.invalidate();
    }

    /// Checks if buffer has complete event packets.
    pub(self) fn has_complete_packets(&self) -> bool {
        self.packet_count.load(Ordering::Relaxed) > 0
    }

    /// Increments the packet count.
    pub(super) fn increment_packet_count(&self) {
        self.packet_count.fetch_add(1, Ordering::Relaxed);
        self.pollee.notify(IoEvents::IN);
    }

    /// Decrements the packet count.
    pub(self) fn decrement_packet_count(&self) {
        if self.packet_count.fetch_sub(1, Ordering::Relaxed) == 1 {
            self.pollee.invalidate();
        }
    }
}

impl EvdevFile {
    pub(super) fn new(
        buffer_size: usize,
        evdev: Weak<EvdevDevice>,
    ) -> (Self, RbProducer<EvdevEvent>) {
        let (producer, consumer) = RingBuffer::new(buffer_size).split();

        let inner = EvdevFileInner {
            consumer: Mutex::new(consumer),
            clock_id: AtomicEvdevClock::new(EvdevClock::Monotonic),
            packet_count: AtomicUsize::new(0),
            pollee: Pollee::new(),
        };
        let evdev_file = Self {
            inner: Arc::new(inner),
            evdev,
        };
        (evdev_file, producer)
    }

    pub(super) fn inner(&self) -> &Arc<EvdevFileInner> {
        &self.inner
    }

    /// Processes events and writes them to the writer.
    ///
    /// Returns the total number of bytes written, or [`Errno::EAGAIN`] if no events are available.
    fn process_events(&self, max_events: usize, writer: &mut VmWriter) -> Result<usize> {
        const EVENT_SIZE: usize = size_of::<EvdevEvent>();

        let mut consumer = self.inner.consumer.lock();
        let mut event_count = 0;

        for _ in 0..max_events {
            let Some(event) = consumer.pop() else {
                break;
            };

            // Update the counter since the event has been consumed.
            if is_syn_report_event(&event) || is_syn_dropped_event(&event) {
                self.inner.decrement_packet_count();
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

        if self.inner.has_complete_packets() {
            IoEvents::IN
        } else {
            IoEvents::empty()
        }
    }

    fn upgrade_evdev_device(&self) -> Result<Arc<EvdevDevice>> {
        self.evdev
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::ENODEV, "evdev device is unavailable"))
    }

    fn handle_evdev_ioctl(&self, raw: u32, arg: usize) -> Result<()> {
        let request = IoctlRequest::decode(raw, arg)?;

        if let Ok(ioctl) = GetVersion::try_from(request) {
            const EVDEV_DRIVER_VERSION: i32 = 0x010001;
            write_val_to_userspace(ioctl.user_ptr(), EVDEV_DRIVER_VERSION)?;
            Ok(())
        } else if let Ok(ioctl) = GetId::try_from(request) {
            let evdev = self.upgrade_evdev_device()?;
            let input_id = evdev.device.id();
            write_val_to_userspace(ioctl.user_ptr(), input_id)?;
            Ok(())
        } else if let Ok(ioctl) = SetClockId::try_from(request) {
            let clock_id_raw: i32 = read_val_from_userspace(ioctl.user_ptr())?;
            let clock_id = ClockId::try_from(clock_id_raw)
                .map_err(|_| Error::with_message(Errno::EINVAL, "invalid clock id"))?;
            let evdev_clock = match clock_id {
                ClockId::CLOCK_REALTIME => EvdevClock::Realtime,
                ClockId::CLOCK_MONOTONIC => EvdevClock::Monotonic,
                ClockId::CLOCK_BOOTTIME => EvdevClock::Boottime,
                _ => {
                    return_errno_with_message!(Errno::EINVAL, "unsupported clock id");
                }
            };
            self.inner.clock_id.store(evdev_clock, Ordering::Relaxed);
            Ok(())
        } else if let Ok(ioctl) = GetDeviceName::try_from(request) {
            let evdev = self.upgrade_evdev_device()?;
            write_string_to_userspace(ioctl.user_ptr(), evdev.device.name(), ioctl.buffer_len())?;
            Ok(())
        } else if let Ok(ioctl) = GetPhys::try_from(request) {
            let evdev = self.upgrade_evdev_device()?;
            write_string_to_userspace(ioctl.user_ptr(), evdev.device.phys(), ioctl.buffer_len())?;
            Ok(())
        } else if let Ok(ioctl) = GetUniq::try_from(request) {
            let evdev = self.upgrade_evdev_device()?;
            write_string_to_userspace(ioctl.user_ptr(), evdev.device.uniq(), ioctl.buffer_len())?;
            Ok(())
        } else if let Ok(ioctl) = GetBit::try_from(request) {
            let evdev = self.upgrade_evdev_device()?;
            let capability = evdev.device.capability();
            let bitmap: &[u8] = match ioctl.event_type() {
                0 => {
                    let event_types_bytes = capability.event_types_bits().to_le_bytes();
                    write_bitmap_to_userspace(
                        ioctl.user_ptr(),
                        &event_types_bytes,
                        ioctl.buffer_len(),
                    )?;
                    return Ok(());
                }
                t if t == EventTypes::KEY.as_index() => capability.supported_keys_bitmap(),
                t if t == EventTypes::REL.as_index() => capability.supported_relative_axes_bitmap(),
                _ => {
                    return_errno_with_message!(Errno::EINVAL, "unsupported event type");
                }
            };
            write_bitmap_to_userspace(ioctl.user_ptr(), bitmap, ioctl.buffer_len())?;
            Ok(())
        } else if let Ok(ioctl) = GetKeyState::try_from(request) {
            let zero = vec![0u8; ioctl.buffer_len()];
            write_bitmap_to_userspace(ioctl.user_ptr(), &zero[..], ioctl.buffer_len())?;
            Ok(())
        } else if let Ok(ioctl) = GetLedState::try_from(request) {
            let zero = vec![0u8; ioctl.buffer_len()];
            write_bitmap_to_userspace(ioctl.user_ptr(), &zero[..], ioctl.buffer_len())?;
            Ok(())
        } else if let Ok(ioctl) = GetSwState::try_from(request) {
            let zero = vec![0u8; ioctl.buffer_len()];
            write_bitmap_to_userspace(ioctl.user_ptr(), &zero[..], ioctl.buffer_len())?;
            Ok(())
        } else {
            Err(Error::with_message(
                Errno::EINVAL,
                "This IOCTL operation not supported on evdev devices",
            ))
        }
    }
}

/// Checks if the event is a `SYN_REPORT` event.
pub(super) fn is_syn_report_event(event: &EvdevEvent) -> bool {
    event.type_ == EventTypes::SYN.as_index() && event.code == SynEvent::Report as u16
}

/// Checks if the event is a `SYN_DROPPED` event.
pub(super) fn is_syn_dropped_event(event: &EvdevEvent) -> bool {
    event.type_ == EventTypes::SYN.as_index() && event.code == SynEvent::Dropped as u16
}

fn read_val_from_userspace<T: Pod>(user_ptr: usize) -> Result<T> {
    current_userspace!().read_val(user_ptr)
}

fn write_val_to_userspace<T: Pod>(user_ptr: usize, value: T) -> Result<()> {
    current_userspace!().write_val(user_ptr, &value)
}

fn write_string_to_userspace(user_ptr: usize, value: &str, len: usize) -> Result<()> {
    if len == 0 {
        return Ok(());
    }

    let bytes = value.as_bytes();
    let copy_len = cmp::min(bytes.len(), len - 1);
    if copy_len > 0 {
        let mut reader = VmReader::from(&bytes[..copy_len]);
        current_userspace!().write_bytes(user_ptr, &mut reader)?;
    }

    let remaining = len - copy_len;
    if remaining > 0 {
        current_userspace!()
            .writer(user_ptr + copy_len, remaining)?
            .fill_zeros(remaining)?;
    }

    Ok(())
}

fn write_bitmap_to_userspace(user_ptr: usize, bitmap: &[u8], len: usize) -> Result<()> {
    if len == 0 {
        return Ok(());
    }

    let copy_len = cmp::min(bitmap.len(), len);
    if copy_len > 0 {
        let mut reader = VmReader::from(&bitmap[..copy_len]);
        current_userspace!().write_bytes(user_ptr, &mut reader)?;
    }

    let remaining = len - copy_len;
    if remaining > 0 {
        current_userspace!()
            .writer(user_ptr + copy_len, remaining)?
            .fill_zeros(remaining)?;
    }

    Ok(())
}

impl Pollable for EvdevFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.inner
            .pollee
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
        if is_nonblocking && !self.inner.has_complete_packets() {
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

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            IoctlCmd::Others(raw) => self.handle_evdev_ioctl(raw, arg)?,
            _ => {
                return_errno!(Errno::EINVAL)
            }
        }

        Ok(0)
    }
}

impl Drop for EvdevFile {
    fn drop(&mut self) {
        if let Some(evdev) = self.evdev.upgrade() {
            evdev.detach_closed_file(&self.inner);
        }
    }
}
