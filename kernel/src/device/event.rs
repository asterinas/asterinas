#![expect(unused_variables)]

use super::*;
use crate::{
    events::IoEvents,
    fs::inode_handle::FileIo,
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::MultiWrite,
};
use aster_input::{register_handler, unregister_handler, InputHandler, InputEvent, event_type_codes::EventType};
use alloc::collections::VecDeque;
use spin::{Mutex, Once};
use aster_input::InputDevice;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct InputEventLinux {
    pub sec: u64,    // Seconds (time.tv_sec or __sec)
    pub usec: u64,   // Microseconds (time.tv_usec or __usec)
    pub type_: u16,  // Event type
    pub code: u16,   // Event code
    pub value: i32,  // Event value
}

impl InputEventLinux {
    pub fn to_bytes(&self) -> [u8; 24] {
        let mut bytes = [0u8; 24];
        bytes[..8].copy_from_slice(&self.sec.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.usec.to_le_bytes());
        bytes[16..18].copy_from_slice(&self.type_.to_le_bytes());
        bytes[18..20].copy_from_slice(&self.code.to_le_bytes());
        bytes[20..24].copy_from_slice(&self.value.to_le_bytes());
        bytes
    }
}

pub struct EventDevice {
    id: usize,
    event_queue: Arc<Mutex<VecDeque<InputEventLinux>>>,
    input_device: Arc<dyn InputDevice>,
}

static EVENT_DEVICE_HANDLER: Once<Arc<EventDeviceHandler>> = Once::new();

impl EventDevice {
    pub fn new(id: usize, input_device: Arc<dyn InputDevice>) -> Arc<Self> {
        let event_device = Arc::new(Self {
            id,
            event_queue: Arc::new(Mutex::new(VecDeque::new())),
            input_device: input_device.clone(),
        });

        // Initialize the static handler if it hasn't been initialized yet
        let handler = EVENT_DEVICE_HANDLER.call_once(|| {
            Arc::new(EventDeviceHandler {
                event_device: Mutex::new(Weak::new()), // Initialize the Mutex
            })
        });

        // Update the handler's weak reference to point to the new EventDevice
        *handler.event_device.lock() = Arc::downgrade(&event_device);

        // Register the handler
        register_handler(handler.clone());

        // Connect the input_device to the handler
        aster_input::acquire_connection(input_device, handler.clone());

        event_device
    }

    pub fn push_event(&self, event: InputEventLinux) {
        let mut queue = self.event_queue.lock();
        if queue.len() >= 128 {
            queue.pop_front();
        }
        queue.push_back(event);
    }

    pub fn input_device(&self) -> Arc<dyn InputDevice> {
        Arc::clone(&self.input_device)
    }
}

impl Clone for EventDevice {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            event_queue: Arc::clone(&self.event_queue),
            input_device: Arc::clone(&self.input_device), // Add this line
        }
    }
}

impl Device for EventDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(13, self.id as u32)
    }

    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        Ok(Some(Arc::new(self.clone())))
    }
}

impl Pollable for EventDevice {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        let queue = self.event_queue.lock();
        let events = if !queue.is_empty() {
            IoEvents::IN
        } else {
            IoEvents::empty()
        };
        events & mask
    }
}

impl FileIo for EventDevice {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        let mut queue = self.event_queue.lock(); // Lock the event queue for thread-safe access
        if let Some(event) = queue.pop_front() { // Retrieve the oldest event from the queue
            let event_bytes = event.to_bytes(); // Serialize the event into bytes
            let mut reader = VmReader::from(&event_bytes[..]); // Create a reader for the serialized bytes
            writer.write(&mut reader)?; // Write the serialized event to the writer
            Ok(event_bytes.len()) // Return the size of the serialized event
        } else {
            Ok(0) // Return 0 if the queue is empty
        }
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        Ok(reader.remain())
    }
}

#[derive(Debug)]
pub struct EventDeviceHandler {
    event_device: Mutex<Weak<EventDevice>>, // Wrap in a Mutex for mutable access
}

impl InputHandler for EventDeviceHandler {
    /// Specifies the event types this handler can process.
    fn supported_event_types(&self) -> Vec<u16> {
        vec![EventType::EvKey as u16, EventType::EvRel as u16] // Supports keyboard and mouse events
    }

    /// Handles the input event by pushing it to the event queue.
    fn handle_event(&self, event: InputEvent) -> core::result::Result<(), core::convert::Infallible> {
        if let Some(event_device) = self.event_device.lock().upgrade() {

            // Convert InputEvent to InputEventLinux
            let linux_event = InputEventLinux {
                sec: event.time / 1_000_000,
                usec: event.time % 1_000_000,
                type_: event.type_,
                code: event.code,
                value: event.value,
            };

            event_device.push_event(linux_event);
        }

        Ok(())
    }
}

impl Drop for EventDevice {
    fn drop(&mut self) {
        // Unregister the handler only if this is the last instance of EventDevice
        if Arc::strong_count(&self.event_queue) == 1 {
            unregister_handler(Arc::new(EventDeviceHandler {
                event_device: Mutex::new(Arc::downgrade(&Arc::new(self.clone()))), // Wrap in Mutex
            }));
        }
    }
}

// Implement the Pollable trait for Arc<EventDevice>
impl Pollable for Arc<EventDevice> {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.as_ref().poll(mask, poller)
    }
}

// Implement the FileIo trait for Arc<EventDevice>
impl FileIo for Arc<EventDevice> {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        // Lock the event queue for thread-safe access
        let mut queue = self.event_queue.lock();
        
        // Retrieve the oldest event from the queue
        if let Some(event) = queue.pop_front() {
            // Serialize the event into bytes
            let event_bytes = event.to_bytes(); // Use the `to_bytes` method of `InputEventLinux`
            
            // Create a reader for the serialized bytes
            let mut reader = VmReader::from(&event_bytes[..]);
            
            // Write the serialized event to the writer
            writer.write(&mut reader)?;
            
            // Return the size of the serialized event
            Ok(event_bytes.len())
        } else {
            // Return 0 if the queue is empty
            Ok(0)
        }
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        self.as_ref().write(reader)
    }
}

// Implement the Device trait for Arc<EventDevice>
impl Device for Arc<EventDevice> {
    fn type_(&self) -> DeviceType {
        self.as_ref().type_()
    }

    fn id(&self) -> DeviceId {
        self.as_ref().id()
    }

    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        self.as_ref().open()
    }
}
