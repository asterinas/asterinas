// SPDX-License-Identifier: MPL-2.0

//! The input devices of Asterinas.
#![no_std]
#![deny(unsafe_code)]
#![feature(fn_traits)]

extern crate alloc;

pub mod event_type_codes;

use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};
use core::{any::Any, fmt::Debug};

use component::{init_component, ComponentInitError};
use ostd::{sync::SpinLock, Pod};
use spin::Once;
use crate::event_type_codes::*;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct InputEvent {
    pub time: u64,    // Timestamp in microseconds
    pub type_: u16,   // Event type (e.g., EV_KEY, EV_REL)
    pub code: u16,    // Event code (e.g., KEY_A, REL_X)
    pub value: i32,   // Event value (e.g., 1 for key press, 0 for key release)
}

impl InputEvent {
    pub fn to_bytes(&self) -> [u8; 16] {
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&self.time.to_le_bytes());
        bytes[8..10].copy_from_slice(&self.type_.to_le_bytes());
        bytes[10..12].copy_from_slice(&self.code.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.value.to_le_bytes());
        bytes
    }
}

struct Connection {
    device: Arc<dyn InputDevice>,   // Reference to the InputDevice
    handler: Arc<dyn InputHandler>, // Reference to the InputHandler
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct InputID {
    pub bustype: u16,       // Bus type
    pub vendor_id: u16,     // Vendor ID
    pub product_id: u16,    // Product ID
    pub version: u16,       // Version of the device
}
#[derive(Debug, Clone)]
pub struct InputDeviceMeta {
    pub name: String,       // Name of the device
    pub phys: String,       // Physical location of the device
    pub uniq: String,       // Unique string of the device
    pub version: u32,       // Version of the device
    pub id: InputID,        // Input_id of the device
}

pub trait InputDevice: Send + Sync + Any {
    fn metadata(&self) -> InputDeviceMeta;

    fn get_prop_bit(&self) -> Vec<PropType>;

    fn get_ev_bit(&self) -> Vec<EventType>;

    fn get_key_bit(&self) -> Vec<KeyEvent>;

    fn get_rel_bit(&self) -> Vec<RelEvent>;

    fn get_msc_bit(&self) -> Vec<MiscEvent>;

    fn get_led_bit(&self) -> Vec<LedEvent>;
}

pub trait InputHandler: Send + Sync {
    /// Returns the event types the handler can process.
    fn supported_event_types(&self) -> Vec<u16>;

    /// Processes the given event.
    fn handle_event(&self, event: InputEvent, str: &str) -> Result<(), core::convert::Infallible>;
}

struct Component {
    input_device_table: SpinLock<BTreeMap<String, Arc<dyn InputDevice>>>, // Manages input devices
    connections: SpinLock<Vec<Connection>>,                              // Manages connections
    input_handlers: SpinLock<Vec<Arc<dyn InputHandler>>>,                // Manages input handlers
}

impl Component {
    pub fn init() -> Result<Self, ComponentInitError> {
        Ok(Self {
            input_device_table: SpinLock::new(BTreeMap::new()),
            connections: SpinLock::new(Vec::new()),
            input_handlers: SpinLock::new(Vec::new()),
        })
    }

    pub fn register_device(&self, name: String, device: Arc<dyn InputDevice>) {
        self.input_device_table.lock().insert(name, device);
    }

    pub fn get_device(&self, name: &str) -> Option<Arc<dyn InputDevice>> {
        self.input_device_table.lock().get(name).cloned()
    }

    pub fn all_devices(&self) -> Vec<(String, Arc<dyn InputDevice>)> {
        self.input_device_table
            .lock()
            .iter()
            .map(|(name, device)| (name.clone(), device.clone()))
            .collect()
    }

    pub fn acquire_connection(
        &self,
        device: Arc<dyn InputDevice>,
        handler: Arc<dyn InputHandler>,
    ) {
        let mut connections = self.connections.lock();

        if connections.iter().any(|conn| Arc::ptr_eq(&conn.device, &device) && Arc::ptr_eq(&conn.handler, &handler)) {
            return;
        }

        connections.push(Connection { device, handler });
    }

    pub fn release_connection(&self, device: Arc<dyn InputDevice>, handler: Arc<dyn InputHandler>) {
        let mut connections = self.connections.lock();

        if let Some(pos) = connections.iter().position(|conn| {
            Arc::ptr_eq(&conn.device, &device) && Arc::ptr_eq(&conn.handler, &handler)
        }) {
            connections.remove(pos);
        }
    }

    pub fn register_handler(&self, handler: Arc<dyn InputHandler>) {
        self.input_handlers.lock().push(handler);
    }

    pub fn unregister_handler(&self, handler: Arc<dyn InputHandler>) {
        let mut handlers = self.input_handlers.lock();
        if let Some(pos) = handlers.iter().position(|h| Arc::ptr_eq(h, &handler)) {
            handlers.remove(pos);
        }
    }

    pub fn input_event(&self, event: InputEvent, str: &str) {
        let connections = self.connections.lock();
        for connection in connections.iter() {
            if connection.device.metadata().name != str {
                continue;
            }
            if connection.handler.supported_event_types().contains(&event.type_) {
                connection.handler.handle_event(event, str).unwrap();
            }
        }
    }
}

static COMPONENT: Once<Component> = Once::new();

#[init_component]
fn component_init() -> Result<(), ComponentInitError> {
    let component = Component::init()?;
    COMPONENT.call_once(|| component);
    Ok(())
}

pub fn register_device(name: String, device: Arc<dyn InputDevice>) {
    COMPONENT.get().unwrap().register_device(name, device);
}

pub fn get_device(name: &str) -> Option<Arc<dyn InputDevice>> {
    COMPONENT.get().unwrap().get_device(name)
}

pub fn all_devices() -> Vec<(String, Arc<dyn InputDevice>)> {
    COMPONENT.get().unwrap().all_devices()
}

pub fn acquire_connection(device: Arc<dyn InputDevice>, handler: Arc<dyn InputHandler>) {
    COMPONENT.get().unwrap().acquire_connection(device, handler);
}

pub fn release_connection(device: Arc<dyn InputDevice>, handler: Arc<dyn InputHandler>) {
    COMPONENT.get().unwrap().release_connection(device, handler);
}

pub fn register_handler(handler: Arc<dyn InputHandler>) {
    COMPONENT.get().unwrap().register_handler(handler);
}

pub fn unregister_handler(handler: Arc<dyn InputHandler>) {
    COMPONENT.get().unwrap().unregister_handler(handler);
}

pub fn input_event(event: InputEvent, str: &str) {
    COMPONENT.get().unwrap().input_event(event, str);
}
