// SPDX-License-Identifier: MPL-2.0

//! Event device (evdev) support.
//!
//! Character device with major number 13. The minor numbers are dynamically allocated.
//! Devices appear as `/dev/input/eventX` where X is the minor number.
//!
//! Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/linux/major.h>

mod file;

use alloc::{
    format,
    string::String,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::{
    fmt::Debug,
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};

use aster_input::{
    event_type_codes::{EventTypes, SynEvent},
    input_dev::{InputDevice, InputEvent},
    input_handler::{ConnectError, InputHandler, InputHandlerClass},
};
use aster_time::read_monotonic_time;
use device_id::{DeviceId, MajorId, MinorId};
use file::{EvdevEvent, EvdevFile, EVDEV_BUFFER_SIZE};
use ostd::sync::SpinLock;
use spin::Once;

use super::char::{acquire_major, register, unregister, CharDevice, MajorIdOwner};
use crate::{
    device::char::DevtmpfsName,
    fs::inode_handle::FileIo,
    prelude::*,
    syscall::ClockId,
    time::clocks::{
        BootTimeClock, MonotonicClock, MonotonicCoarseClock, MonotonicRawClock, RealTimeClock,
        RealTimeCoarseClock,
    },
    util::ring_buffer::RbProducer,
};

/// Major device number for evdev devices.
const EVDEV_MAJOR_ID: u16 = 13;

/// Global minor number allocator for evdev devices.
static EVDEV_MINOR_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Global registry of evdev devices.
static EVDEV_DEVICES: SpinLock<BTreeMap<MinorId, Arc<EvdevDevice>>> =
    SpinLock::new(BTreeMap::new());

pub struct EvdevDevice {
    /// Input device associated with this evdev.
    device: Arc<dyn InputDevice>,
    /// List of opened evdev files with their producers.
    ///
    /// # Deadlock Prevention
    ///
    /// This lock is acquired in both the task and interrupt contexts.
    /// We must make sure that this lock is taken with the local IRQs disabled.
    /// Otherwise, we would be vulnerable to deadlock.
    opened_files: SpinLock<Vec<(Weak<EvdevFile>, RbProducer<EvdevEvent>)>>,
    /// Device node name (e.g., "event0").
    node_name: String,
    /// Device ID.
    id: DeviceId,
}

impl Debug for EvdevDevice {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        let opened_count = self
            .opened_files
            .lock()
            .iter()
            .filter(|(file, _)| file.strong_count() > 0)
            .count();
        f.debug_struct("EvdevDevice")
            .field("minor", &self.id.minor())
            .field("device_name", &self.device.name())
            .field("opened_files", &opened_count)
            .finish()
    }
}

impl EvdevDevice {
    pub(self) fn new(minor: u32, device: Arc<dyn InputDevice>) -> Self {
        let node_name = format!("event{}", minor);
        let major = EVDEV_MAJOR.get().unwrap().get();
        let minor_id = MinorId::new(minor);

        Self {
            device,
            opened_files: SpinLock::new(Vec::new()),
            node_name,
            id: DeviceId::new(major, minor_id),
        }
    }

    /// Checks if this evdev device is associated with the given input device.
    pub(self) fn matches_input_device(&self, input_device: &Arc<dyn InputDevice>) -> bool {
        Arc::ptr_eq(&self.device, input_device)
    }

    /// Adds an opened evdev file to this evdev device.
    fn attach_file(&self, file: Weak<EvdevFile>, producer: RbProducer<EvdevEvent>) {
        let mut opened_files = self.opened_files.disable_irq().lock();
        opened_files.push((file, producer));
    }

    /// Removes closed evdev files from this evdev device.
    pub(self) fn detach_closed_files(&self) {
        let mut opened_files = self.opened_files.disable_irq().lock();
        opened_files.retain(|(file, _)| file.strong_count() > 0);
    }

    /// Distributes events to all opened evdev files.
    pub(self) fn pass_events(&self, events: &[InputEvent]) {
        let mut opened_files = self.opened_files.lock();

        // Send events to all opened evdev files using their producers.
        for (file_weak, producer) in opened_files.iter_mut() {
            let Some(file) = file_weak.upgrade() else {
                continue;
            };

            for event in events {
                // Get time according to the opened evdev file's clock type.
                let time = self.get_time_for_file(&file);
                let timed_event = EvdevEvent::from_event_and_time(event, time);

                // Try to push event to the buffer.
                if let Some(()) = producer.push(timed_event) {
                    // Check if this is a SYN_REPORT event
                    if self.is_syn_report_event(event) {
                        file.increment_packet_count();
                    }
                } else {
                    // TODO: In Linux's implementation, when the buffer is full, evdev will pop the two
                    // oldest events to ensure that both the SYN_DROPPED event and the current event can
                    // be pushed into the buffer. However, since we are using `RingBuffer`, evdev cannot
                    // pop events. Considering the convenience of lock-free programming with `RingBuffer`
                    // and the rarity of this error condition, we choose to retain the use of `RingBuffer`
                    // and instead attempt to push both the SYN_DROPPED event and the current event.
                    let dropped_event = EvdevEvent::from_event_and_time(
                        &InputEvent::from_sync_event(SynEvent::Dropped),
                        time,
                    );

                    // Try to push SYN_DROPPED event.
                    if let Some(()) = producer.push(dropped_event) {
                        file.increment_packet_count();

                        // Try to push the original event.
                        if let Some(()) = producer.push(timed_event) {
                            // Check if this is a SYN_REPORT event.
                            if self.is_syn_report_event(event) {
                                file.increment_packet_count();
                            }
                        }
                    } else {
                        // Failed to push SYN_DROPPED event, emit a warning.
                        log::warn!(
                            "Failed to push SYN_DROPPED event to evdev file buffer (buffer full)"
                        );
                    }
                }
            }
        }
    }

    /// Checks if the event is a SYN_REPORT event (marks end of packet).
    fn is_syn_report_event(&self, event: &InputEvent) -> bool {
        let (type_, code, _) = event.to_raw();
        type_ == EventTypes::SYN.as_index() && code == SynEvent::Report as u16
    }

    /// Gets time according to the opened evdev file's clock ID.
    fn get_time_for_file(&self, file: &EvdevFile) -> Duration {
        let clock_id = file.clock_id();

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

    /// Creates a new opened evdev file for this evdev device.
    pub(self) fn create_file(self: &Arc<Self>, buffer_size: usize) -> Result<Arc<EvdevFile>> {
        // Create the evdev file and get the producer.
        let (file, producer) = EvdevFile::new(buffer_size, Arc::downgrade(self));
        let file = Arc::new(file);
        let file_weak = Arc::downgrade(&file);

        // Attach the opened evdev file to this device.
        self.attach_file(file_weak, producer);

        Ok(file)
    }
}

impl CharDevice for EvdevDevice {
    fn devtmpfs_name(&self) -> DevtmpfsName<'_> {
        DevtmpfsName::new(&self.node_name, Some("input"))
    }

    fn id(&self) -> DeviceId {
        self.id
    }

    fn open(&self) -> Result<Arc<dyn FileIo>> {
        // Get the device from the registry.
        let devices = EVDEV_DEVICES.lock();
        let Some(evdev) = devices.get(&self.id.minor()) else {
            return_errno_with_message!(
                Errno::ENODEV,
                "the evdev device does not exist in the registry"
            );
        };

        // Create a new opened evdev file for this evdev device.
        let file = evdev.create_file(EVDEV_BUFFER_SIZE)?;
        Ok(file as Arc<dyn FileIo>)
    }
}

/// Evdev handler class that creates device nodes for input devices.
#[derive(Debug)]
struct EvdevHandlerClass;

impl InputHandlerClass for EvdevHandlerClass {
    fn name(&self) -> &str {
        "evdev"
    }

    fn connect(
        &self,
        dev: Arc<dyn InputDevice>,
    ) -> core::result::Result<Arc<dyn InputHandler>, ConnectError> {
        // Allocate a new minor number.
        let minor = EVDEV_MINOR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let minor_id = MinorId::new(minor);

        // Create evdev device.
        let evdev = Arc::new(EvdevDevice::new(minor, dev.clone()));

        // Register the device with the char device subsystem.
        register(evdev.clone()).map_err(|_| ConnectError::InternalError)?;

        // Add to our registry for lookup during disconnect.
        EVDEV_DEVICES.lock().insert(minor_id, evdev.clone());

        // Create handler instance for this device.
        let handler = EvdevHandler::new(evdev);
        Ok(Arc::new(handler))
    }

    fn disconnect(&self, dev: &Arc<dyn InputDevice>) {
        let mut devices = EVDEV_DEVICES.lock();
        let device_name = dev.name();

        // Find the device by checking if it matches the input device.
        let mut found_minor = None;
        for (minor, evdev) in devices.iter() {
            if evdev.matches_input_device(dev) {
                found_minor = Some(*minor);
                break;
            }
        }

        if let Some(minor) = found_minor {
            let evdev = devices.remove(&minor).unwrap();
            let device_id = evdev.id();

            // Unregister from the char device subsystem.
            if let Err(e) = unregister(device_id) {
                log::warn!(
                    "Failed to unregister evdev device '{}' (minor: {}): {:?}",
                    device_name,
                    minor.get(),
                    e
                );
            }

            // TODO: Implement device node deletion when the functionality is available.
            log::info!(
                "Disconnected evdev device '{}' (minor: {}), device node /dev/input/event{} still exists",
                device_name,
                minor.get(),
                minor.get()
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
    evdev: Arc<EvdevDevice>,
}

impl EvdevHandler {
    fn new(evdev: Arc<EvdevDevice>) -> Self {
        Self { evdev }
    }
}

impl InputHandler for EvdevHandler {
    fn handle_events(&self, events: &[InputEvent]) {
        self.evdev.pass_events(events);
    }
}

static EVDEV_MAJOR: Once<MajorIdOwner> = Once::new();

pub(super) fn init_in_first_kthread() {
    EVDEV_MAJOR.call_once(|| acquire_major(MajorId::new(EVDEV_MAJOR_ID)).unwrap());

    static REGISTERED_EVDDEV_CLASS: spin::Once<
        aster_input::input_handler::RegisteredInputHandlerClass,
    > = spin::Once::new();

    let handler_class = Arc::new(EvdevHandlerClass);
    let handle = aster_input::register_handler_class(handler_class);
    REGISTERED_EVDDEV_CLASS.call_once(|| handle);

    log::info!("Evdev device support initialized");
}
