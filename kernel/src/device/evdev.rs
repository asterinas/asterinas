// SPDX-License-Identifier: MPL-2.0

use alloc::{
    format,
    string::String,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::{
    fmt::Debug,
    sync::atomic::{AtomicU32, AtomicUsize, Ordering},
    time::Duration,
};

use aster_input::{
    event_type_codes::{EventTypes, SynEvent},
    ConnectError, InputDevice, InputEvent, InputHandler, InputHandlerClass,
};
use aster_time::read_monotonic_time;
use ostd::{
    sync::{Mutex, SpinLock},
    Pod,
};

use crate::{
    events::IoEvents,
    fs::{
        device::{add_node, Device, DeviceId, DeviceType},
        fs_resolver::FsResolver,
        inode_handle::FileIo,
        utils::IoctlCmd,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    syscall::ClockId,
    time::clocks::{
        BootTimeClock, MonotonicClock, MonotonicCoarseClock, MonotonicRawClock, RealTimeClock,
        RealTimeCoarseClock,
    },
    util::ring_buffer::{RbConsumer, RbProducer, RingBuffer},
    VmReader, VmWriter,
};

/// Maximum number of events in the evdev buffer.
const EVDEV_BUFFER_SIZE: usize = 64;

/// Global minor number allocator for evdev devices.
static EVDEV_MINOR_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Global registry of evdev devices.
static EVDEV_DEVICES: SpinLock<Vec<(u32, Arc<Evdev>)>> = SpinLock::new(Vec::new());

/// Global FsResolver for device node creation.
static FS_RESOLVER: SpinLock<Option<Arc<FsResolver>>> = SpinLock::new(None);

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

    pub fn to_bytes(self) -> [u8; core::mem::size_of::<EvdevEvent>()] {
        let mut bytes = [0u8; core::mem::size_of::<EvdevEvent>()];
        bytes[0..core::mem::size_of::<u64>()].copy_from_slice(&self.sec.to_le_bytes());
        bytes[core::mem::size_of::<u64>()..2 * core::mem::size_of::<u64>()]
            .copy_from_slice(&self.usec.to_le_bytes());
        bytes[2 * core::mem::size_of::<u64>()
            ..2 * core::mem::size_of::<u64>() + core::mem::size_of::<u16>()]
            .copy_from_slice(&self.type_.to_le_bytes());
        bytes[2 * core::mem::size_of::<u64>() + core::mem::size_of::<u16>()
            ..2 * core::mem::size_of::<u64>() + 2 * core::mem::size_of::<u16>()]
            .copy_from_slice(&self.code.to_le_bytes());
        bytes[2 * core::mem::size_of::<u64>() + 2 * core::mem::size_of::<u16>()..]
            .copy_from_slice(&self.value.to_le_bytes());
        bytes
    }
}

pub struct EvdevClient {
    /// Consumer for reading events.
    consumer: Mutex<RbConsumer<EvdevEvent>>,
    /// Client-specific clock type.
    clock_type: AtomicU32,
    /// Number of events available.
    event_count: AtomicUsize,
    /// Number of complete event packets available (ended with SYN_REPORT).
    packet_count: AtomicUsize,
    /// Pollee for event notification.
    pollee: Pollee,
    /// Weak reference to the evdev device that owns this client.
    evdev: Weak<Evdev>,
}

impl EvdevClient {
    fn new(buffer_size: usize, evdev: Weak<Evdev>) -> (Self, RbProducer<EvdevEvent>) {
        let (producer, consumer) = RingBuffer::new(buffer_size).split();

        let client = Self {
            consumer: Mutex::new(consumer),
            clock_type: AtomicU32::new(1), // Default to be CLOCK_MONOTONIC
            event_count: AtomicUsize::new(0),
            packet_count: AtomicUsize::new(0),
            pollee: Pollee::new(),
            evdev,
        };
        (client, producer)
    }

    /// Reads events from this client's buffer.
    pub fn read_events(&self, count: usize) -> Vec<EvdevEvent> {
        let mut events = Vec::new();
        let mut consumer = self.consumer.lock();

        for _ in 0..count {
            if let Some(event) = consumer.pop() {
                // Checks if this is a SYN_REPORT event.
                let is_syn_report = self.is_syn_report_event(&event);

                events.push(event);
                self.decrement_event_count();

                if is_syn_report {
                    self.decrement_packet_count();
                }
            } else {
                break;
            }
        }

        events
    }

    /// Checks if the EvdevEvent is a `SYN_REPORT` event.
    fn is_syn_report_event(&self, event: &EvdevEvent) -> bool {
        event.type_ == EventTypes::SYN.as_u16() && event.code == SynEvent::SynReport as u16
    }

    /// Checks if buffer has complete event packets.
    pub fn has_complete_packets(&self) -> bool {
        self.packet_count.load(Ordering::Relaxed) > 0
    }

    /// Increments event count.
    pub fn increment_event_count(&self) {
        self.event_count.fetch_add(1, Ordering::Relaxed);
        self.pollee.notify(IoEvents::IN);
    }

    /// Decrements event count.
    pub fn decrement_event_count(&self) {
        self.event_count.fetch_sub(1, Ordering::Relaxed);
        if self.event_count.load(Ordering::Relaxed) == 0 {
            self.pollee.invalidate();
        }
    }

    /// Increments packet count.
    pub fn increment_packet_count(&self) {
        self.packet_count.fetch_add(1, Ordering::Relaxed);
        self.pollee.notify(IoEvents::IN);
    }

    /// Decrements packet count.
    pub fn decrement_packet_count(&self) {
        self.packet_count.fetch_sub(1, Ordering::Relaxed);
        if self.packet_count.load(Ordering::Relaxed) == 0 {
            self.pollee.invalidate();
        }
    }

    /// Processes events and write them to the writer.
    /// Returns the total number of bytes written, or EAGAIN if no events available.
    fn process_events(&self, max_events: usize, writer: &mut VmWriter) -> Result<usize> {
        const EVENT_SIZE: usize = core::mem::size_of::<EvdevEvent>();

        let events = self.read_events(max_events);
        if events.is_empty() {
            return Err(Error::with_message(Errno::EAGAIN, "No events available"));
        }

        // Writes all events to the buffer.
        let mut total_bytes = 0;
        for event in events {
            let event_bytes = event.to_bytes();
            writer.write_fallible(&mut event_bytes.as_slice().into())?;
            total_bytes += EVENT_SIZE;
        }

        Ok(total_bytes)
    }
}

impl Pollable for EvdevClient {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee.poll_with(mask, poller, || {
            let has_complete_packets = self.has_complete_packets();

            let mut events = IoEvents::empty();
            if has_complete_packets && mask.contains(IoEvents::IN) {
                events |= IoEvents::IN;
            }

            events
        })
    }
}

impl FileIo for EvdevClient {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        let requested_bytes = writer.avail();
        let max_events = requested_bytes / core::mem::size_of::<EvdevEvent>();

        if max_events == 0 {
            return Ok(0);
        }

        match self.process_events(max_events, writer) {
            Ok(bytes) => Ok(bytes),
            Err(e) if e.error() == Errno::EAGAIN => self.wait_events(IoEvents::IN, None, || {
                self.process_events(max_events, writer)
            }),
            Err(e) => Err(e),
        }
    }

    fn write(&self, _reader: &mut VmReader) -> Result<usize> {
        // TODO: support write operation on evdev devices
        Err(Error::with_message(
            Errno::ENOSYS,
            "WRITE operation not supported on evdev devices",
        ))
    }

    fn ioctl(&self, _cmd: IoctlCmd, _arg: usize) -> Result<i32> {
        // TODO: support ioctl operation on evdev devices
        Err(Error::with_message(
            Errno::EINVAL,
            "IOCTL operation not supported on evdev devices",
        ))
    }
}

impl Debug for EvdevClient {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("EvdevClient")
            .field("event_count", &self.event_count.load(Ordering::Relaxed))
            .field("clock_type", &self.clock_type.load(Ordering::Relaxed))
            .finish()
    }
}

impl Drop for EvdevClient {
    fn drop(&mut self) {
        if let Some(evdev) = self.evdev.upgrade() {
            evdev.detach_client();
            evdev.close_device();
        }
    }
}

pub struct Evdev {
    /// Minor device number.
    minor: u32,
    /// Reference count of open clients.
    open: SpinLock<u32>,
    /// Input device associated with this evdev.
    device: Arc<dyn InputDevice>,
    /// List of clients with their producers.
    client_list: SpinLock<Vec<(Arc<EvdevClient>, RbProducer<EvdevEvent>)>>,
    /// Device name.
    device_name: String,
}

impl Debug for Evdev {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Evdev")
            .field("minor", &self.minor)
            .field("device_name", &self.device_name)
            .field("open", &self.open)
            .finish()
    }
}

impl Evdev {
    fn new(minor: u32, device: Arc<dyn InputDevice>) -> Self {
        let device_name = device.name().to_string();
        Self {
            minor,
            open: SpinLock::new(0),
            device,
            client_list: SpinLock::new(Vec::new()),
            device_name,
        }
    }

    /// Checks if this evdev device is associated with the given input device.
    pub fn matches_input_device(&self, input_device: &Arc<dyn InputDevice>) -> bool {
        Arc::ptr_eq(&self.device, input_device)
    }

    /// Adds a client to this evdev device.
    pub fn attach_client(&self, client: Arc<EvdevClient>, producer: RbProducer<EvdevEvent>) {
        let mut client_list = self.client_list.lock();
        client_list.push((client, producer));
    }

    /// Removes closed clients from this evdev device.
    pub fn detach_client(&self) {
        let mut client_list = self.client_list.lock();
        client_list.retain(|(client, _)| Arc::strong_count(client) > 1);
    }

    /// Distributes events to all clients.
    pub fn pass_events(&self, events: &[InputEvent]) {
        let mut client_list = self.client_list.lock();

        // Sends events to all clients using their producers.
        for (client, producer) in client_list.iter_mut() {
            for event in events {
                // Gets time according to client's clock type.
                let time = self.get_time_for_client(client);
                let timed_event = EvdevEvent::from_event_and_time(event, time);

                // Tries to push event to the buffer.
                if let Some(()) = producer.push(timed_event) {
                    client.increment_event_count();

                    // Check if this is a SYN_REPORT event
                    if self.is_syn_report_event(event) {
                        client.increment_packet_count();
                    }
                } else {
                    // TODO: In Linux's implementation, when the buffer is full, evdev will pop the two
                    // oldest events to ensure that both the SYN_DROPPED event and the current event can
                    // be pushed into the buffer. However, since we are using `RingBuffer`, evdev cannot
                    // pop events. Considering the convenience of lock-free programming with `RingBuffer`
                    // and the rarity of this error condition, we choose to retain the use of `RingBuffer`
                    // and instead attempt to push both the SYN_DROPPED event and the current event.

                    let dropped_event = EvdevEvent::from_event_and_time(
                        &InputEvent::sync(aster_input::event_type_codes::SynEvent::SynDropped),
                        time,
                    );

                    // Tries to push SYN_DROPPED event (this might also fail if buffer is still full)
                    if let Some(()) = producer.push(dropped_event) {
                        client.increment_event_count();
                        client.increment_packet_count();

                        // Tries to push the original event.
                        if let Some(()) = producer.push(timed_event) {
                            client.increment_event_count();

                            // Checks if this is a SYN_REPORT event.
                            if self.is_syn_report_event(event) {
                                client.increment_packet_count();
                            }
                        }
                    }
                }
            }
        }
    }

    /// Checks if the event is a SYN_REPORT event (marks end of packet).
    fn is_syn_report_event(&self, event: &InputEvent) -> bool {
        let (type_, code, _) = event.to_raw();
        type_ == EventTypes::SYN.as_u16() && code == SynEvent::SynReport as u16
    }

    /// Gets time according to client's clock type.
    fn get_time_for_client(&self, client: &EvdevClient) -> Duration {
        let clock_type = client.clock_type.load(Ordering::Relaxed);
        let clock_id = ClockId::try_from(clock_type as i32).unwrap_or(ClockId::CLOCK_MONOTONIC);

        match clock_id {
            ClockId::CLOCK_REALTIME => RealTimeClock::get().read_time(),
            ClockId::CLOCK_MONOTONIC => MonotonicClock::get().read_time(),
            ClockId::CLOCK_MONOTONIC_RAW => MonotonicRawClock::get().read_time(),
            ClockId::CLOCK_REALTIME_COARSE => RealTimeCoarseClock::get().read_time(),
            ClockId::CLOCK_MONOTONIC_COARSE => MonotonicCoarseClock::get().read_time(),
            ClockId::CLOCK_BOOTTIME => BootTimeClock::get().read_time(),
            // For process/thread clocks, fallback to monotonic time.
            ClockId::CLOCK_PROCESS_CPUTIME_ID | ClockId::CLOCK_THREAD_CPUTIME_ID => {
                read_monotonic_time()
            }
        }
    }

    /// Opens the device.
    pub fn open_device(&self) {
        let mut open = self.open.lock();
        *open += 1;
    }

    /// Closes the device.
    pub fn close_device(&self) {
        let mut open = self.open.lock();
        if *open > 0 {
            *open -= 1;
        }
    }

    /// Creates a new client for this evdev device.
    pub fn create_client(self: &Arc<Self>, buffer_size: usize) -> Result<Arc<dyn FileIo>> {
        self.open_device();

        // Creates the client and gets the producer.
        let (client, producer) = EvdevClient::new(buffer_size, Arc::downgrade(self));
        let client = Arc::new(client);

        // Attaches the client to this device.
        self.attach_client(client.clone(), producer);

        Ok(client as Arc<dyn FileIo>)
    }
}

impl Device for Evdev {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        // The same value as Linux.
        DeviceId::new(13, self.minor)
    }

    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        let devices = EVDEV_DEVICES.lock();
        if let Some((_, evdev)) = devices.iter().find(|(minor, _)| *minor == self.minor) {
            // Creates a new client for this evdev device.
            match evdev.create_client(EVDEV_BUFFER_SIZE) {
                Ok(client) => Ok(Some(client)),
                Err(e) => Err(e),
            }
        } else {
            return_errno_with_message!(Errno::ENODEV, "evdev device not found in registry");
        }
    }
}

impl Pollable for Evdev {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        // This shouldn't be called directly.
        // Evdev devices are read-only, so never indicate writable.
        IoEvents::empty()
    }
}

impl FileIo for Evdev {
    fn read(&self, _writer: &mut VmWriter) -> Result<usize> {
        // This shouldn't be called directly since we return a different FileIo in `open()`.
        return_errno_with_message!(Errno::ENODEV, "direct read on evdev device not supported");
    }

    fn write(&self, _reader: &mut VmReader) -> Result<usize> {
        // This shouldn't be called directly since we return a different FileIo in `open()`.
        return_errno_with_message!(Errno::ENODEV, "direct write on evdev device not supported");
    }

    fn ioctl(&self, _cmd: IoctlCmd, _arg: usize) -> Result<i32> {
        // This shouldn't be called directly since we return a different FileIo in `open()`.
        return_errno_with_message!(Errno::ENODEV, "direct ioctl on evdev device not supported");
    }
}

/// Evdev handler class that creates device nodes for input devices.
#[derive(Debug)]
pub struct EvdevHandlerClass {
    name: String,
}

impl EvdevHandlerClass {
    pub fn new() -> Self {
        Self {
            name: "evdev".to_string(),
        }
    }
}

impl InputHandlerClass for EvdevHandlerClass {
    fn name(&self) -> &str {
        &self.name
    }

    fn connect(
        &self,
        dev: Arc<dyn InputDevice>,
    ) -> core::result::Result<Arc<dyn InputHandler>, ConnectError> {
        // Allocates a new minor number.
        let minor = EVDEV_MINOR_COUNTER.fetch_add(1, Ordering::Relaxed);

        // Creates evdev device.
        let evdev = Arc::new(Evdev::new(minor, dev.clone()));
        let device_path = format!("input/event{}", minor);

        // Creates the device node.
        let fs_resolver = FS_RESOLVER
            .lock()
            .clone()
            .ok_or(ConnectError::InternalError)?;
        match add_node(evdev.clone(), &device_path, &fs_resolver) {
            Ok(_) => {
                EVDEV_DEVICES.lock().push((minor, evdev.clone()));

                // Creates handler instance for this device.
                let handler = EvdevHandler::new(evdev);
                Ok(Arc::new(handler))
            }
            Err(_err) => Err(ConnectError::DeviceNodeCreationFailed),
        }
    }

    fn disconnect(&self, dev: &Arc<dyn InputDevice>) {
        let mut devices = EVDEV_DEVICES.lock();
        let device_name = dev.name();

        // Finds the device by checking if it matches the input device.
        if let Some(pos) = devices
            .iter()
            .position(|(_, evdev)| evdev.matches_input_device(dev))
        {
            let (minor, _) = devices.remove(pos);
            let device_path = format!("input/event{}", minor);

            // TODO: Implement device node deletion when the functionality is available.
            log::info!(
                "Disconnected evdev device '{}' (minor: {}), device node /dev/{} still exists",
                device_name,
                minor,
                device_path
            );
        } else {
            log::warn!(
                "Attempted to disconnect device '{}' but it did not connect to evdev",
                device_name
            );
        }
    }
}

/// Evdev handler instance for a specific device.
#[derive(Debug)]
pub struct EvdevHandler {
    evdev: Arc<Evdev>,
}

impl EvdevHandler {
    fn new(evdev: Arc<Evdev>) -> Self {
        Self { evdev }
    }
}

impl InputHandler for EvdevHandler {
    fn class_name(&self) -> &str {
        "evdev"
    }

    fn handle_event(&self, event: &InputEvent) {
        // Creates a single-element array and calls `handle_events`.
        self.handle_events(&[*event]);
    }

    fn handle_events(&self, events: &[InputEvent]) {
        self.evdev.pass_events(events);
    }
}

/// Initializes evdev support in the kernel device system.
pub fn init(fs_resolver: &FsResolver) -> Result<()> {
    // Stores the FsResolver for use in device node creation.
    FS_RESOLVER.lock().replace(Arc::new(fs_resolver.clone()));

    let handler_class = Arc::new(EvdevHandlerClass::new());

    // Registers the evdev handler class with the input subsystem.
    aster_input::register_handler_class(handler_class);

    log::info!("Evdev device support initialized");
    Ok(())
}
