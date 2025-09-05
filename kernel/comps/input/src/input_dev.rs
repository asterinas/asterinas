// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};
use core::{any::Any, fmt::Debug};

use ostd::{sync::RwLock, Pod};

use crate::{
    event_type_codes::{EventTypes, KeyCode, KeyCodeMap, KeyStatus, RelCode, RelCodeMap, SynEvent},
    input_handler::InputHandler,
    unregister_device,
};

/// For now we only implement `EV_SYN`, `EV_KEY`, `EV_REL`. Other types are TODO.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    /// Synchronization events (EV_SYN)
    Sync(SynEvent),
    /// Key press/release events (EV_KEY)
    Key(KeyCode, KeyStatus),
    /// Relative movement events (EV_REL)
    Relative(RelCode, i32),
    // TODO: Add EV_ABS, EV_MSC, EV_SW, EV_LED, EV_SND, ... as needed
}

impl InputEvent {
    /// Creates a synchronization event.
    pub fn sync(sync_type: SynEvent) -> Self {
        Self::Sync(sync_type)
    }

    /// Creates a key event.
    pub fn key(key: KeyCode, status: KeyStatus) -> Self {
        Self::Key(key, status)
    }

    /// Creates a relative movement event.
    pub fn relative(axis: RelCode, value: i32) -> Self {
        Self::Relative(axis, value)
    }

    /// Converts enum to raw Linux input event triplet (type, code, value).
    pub fn to_raw(&self) -> (u16, u16, i32) {
        match self {
            InputEvent::Sync(sync_type) => (
                EventTypes::SYN.as_u16(),
                *sync_type as u16,
                0, // Sync events always have value = 0
            ),
            InputEvent::Key(key, status) => (EventTypes::KEY.as_u16(), *key as u16, *status as i32),
            InputEvent::Relative(axis, value) => (EventTypes::REL.as_u16(), *axis as u16, *value),
        }
    }

    /// Gets the event type.
    pub fn event_type(&self) -> EventTypes {
        match self {
            InputEvent::Sync(_) => EventTypes::SYN,
            InputEvent::Key(_, _) => EventTypes::KEY,
            InputEvent::Relative(_, _) => EventTypes::REL,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct InputId {
    bustype: u16, // Bus type
    vendor: u16,  // Vendor ID
    product: u16, // Product ID
    version: u16, // Version number
}

impl InputId {
    /// Creates a new InputId with the specified values.
    pub fn new(bustype: u16, vendor: u16, product: u16, version: u16) -> Self {
        Self {
            bustype,
            vendor,
            product,
            version,
        }
    }

    /// Gets the bus type.
    pub fn bustype(&self) -> u16 {
        self.bustype
    }

    /// Gets the vendor ID.
    pub fn vendor(&self) -> u16 {
        self.vendor
    }

    /// Gets the product ID.
    pub fn product(&self) -> u16 {
        self.product
    }

    /// Gets the version number.
    pub fn version(&self) -> u16 {
        self.version
    }

    /// Common bus types.
    pub const BUS_PCI: u16 = 0x01;
    pub const BUS_ISAPNP: u16 = 0x02;
    pub const BUS_USB: u16 = 0x03;
    pub const BUS_HIL: u16 = 0x04;
    pub const BUS_BLUETOOTH: u16 = 0x05;
    pub const BUS_VIRTUAL: u16 = 0x06;
    pub const BUS_ISA: u16 = 0x10;
    pub const BUS_I8042: u16 = 0x11;
    pub const BUS_XTKBD: u16 = 0x12;
    pub const BUS_RS232: u16 = 0x13;
    pub const BUS_GAMEPORT: u16 = 0x14;
    pub const BUS_PARPORT: u16 = 0x15;
    pub const BUS_AMIGA: u16 = 0x16;
    pub const BUS_ADB: u16 = 0x17;
    pub const BUS_I2C: u16 = 0x18;
    pub const BUS_HOST: u16 = 0x19;
    pub const BUS_GSC: u16 = 0x1A;
    pub const BUS_ATARI: u16 = 0x1B;
    pub const BUS_SPI: u16 = 0x1C;
    pub const BUS_RMI: u16 = 0x1D;
    pub const BUS_CEC: u16 = 0x1E;
    pub const BUS_INTEL_ISHTP: u16 = 0x1F;
}

/// Input device capability bitmaps.
#[derive(Debug, Clone)]
pub struct InputCapability {
    /// Supported event types (`EV_KEY`, `EV_REL`, etc.)
    supported_event_types: EventTypes,
    /// Supported key/button codes.
    supported_keys: KeyCodeMap,
    /// Supported relative axis codes.
    supported_relative_axes: RelCodeMap,
    // TODO: Add supported_absolute_axes, supported_misc, etc.
}

impl Default for InputCapability {
    fn default() -> Self {
        Self::new()
    }
}

impl InputCapability {
    pub fn new() -> Self {
        Self {
            supported_event_types: EventTypes::new(),
            supported_keys: KeyCodeMap::new(),
            supported_relative_axes: RelCodeMap::new(),
        }
    }

    /// Sets event type capability.
    pub fn set_supported_event_type(&mut self, event_type: EventTypes) {
        self.supported_event_types |= event_type;
    }

    /// Checks if an event type is supported.
    pub fn support_event_type(&self, event_type: EventTypes) -> bool {
        self.supported_event_types.contains(event_type)
    }

    /// Removes support for an event type.
    pub fn clear_supported_event_type(&mut self, event_type: EventTypes) {
        self.supported_event_types &= !event_type;
    }

    /// Sets key capability.
    pub fn set_supported_key(&mut self, key_code: KeyCode) {
        self.supported_keys.set(key_code);
        self.set_supported_event_type(EventTypes::KEY);
    }

    /// Checks if a key code is supported.
    pub fn support_key(&self, key_code: KeyCode) -> bool {
        self.supported_keys.contains(key_code)
    }

    /// Clears a key capability.
    pub fn clear_supported_key(&mut self, key_code: KeyCode) {
        self.supported_keys.clear(key_code);
    }

    /// Sets relative axis capability.
    pub fn set_supported_relative_axis(&mut self, rel_code: RelCode) {
        self.supported_relative_axes.set(rel_code);
        self.set_supported_event_type(EventTypes::REL);
    }

    /// Checks if a relative code is supported.
    pub fn support_relative_axis(&self, rel_code: RelCode) -> bool {
        self.supported_relative_axes.contains(rel_code)
    }

    /// Clears a relative capability.
    pub fn clear_supported_relative_axis(&mut self, rel_code: RelCode) {
        self.supported_relative_axes.clear(rel_code);
    }
}

pub trait InputDevice: Send + Sync + Any + Debug {
    /// Device name.
    fn name(&self) -> &str;

    /// Physical location of the device in the system topology.
    ///
    /// This string describes the physical path through which the device is connected
    /// to the system. It helps identify where the device is physically located and
    /// how it's connected (e.g., USB port, ISA bus, etc.).
    ///
    /// # Examples
    /// - `"isa0060/serio0/input0"` - i8042 keyboard connected via ISA bus
    /// - `"usb-0000:00:1d.0-1/input0"` - USB device connected to specific USB port
    fn phys(&self) -> &str;

    /// Unique identifier for the device instance.
    ///
    /// This string provides a unique identifier for the specific device instance,
    /// typically derived from device-specific information like serial numbers,
    /// MAC addresses, or other hardware-level unique identifiers.
    ///
    /// # Examples
    /// - `"00:1B:DC:0F:AC:27"` - MAC address for Bluetooth devices
    /// - `"S/N: 12345678"` - Device serial number
    /// - `""` - Empty string for devices without unique identifiers
    fn uniq(&self) -> &str;

    /// Device ID.
    fn id(&self) -> InputId;

    /// Device capabilities.
    fn capability(&self) -> &InputCapability;
}

/// Registered input device that can submit events to handlers.
#[derive(Debug)]
pub struct RegisteredInputDevice {
    /// Original device.
    device: Arc<dyn InputDevice>,
    /// Reference to handlers for direct event dispatch.
    handlers: Arc<RwLock<Vec<Arc<dyn InputHandler>>>>,
}

impl RegisteredInputDevice {
    pub(crate) fn new(
        device: Arc<dyn InputDevice>,
        handlers: Arc<RwLock<Vec<Arc<dyn InputHandler>>>>,
    ) -> Self {
        Self { device, handlers }
    }

    /// Submits a single event to all handlers.
    pub fn submit_event(&self, event: &InputEvent) {
        // Check if this device supports the event type
        if !self.is_event_supported(event) {
            log::warn!(
                "Device '{}' does not support event {:?}, dropping event",
                self.device.name(),
                event
            );
            return;
        }

        let handlers = self.handlers.read();
        if handlers.is_empty() {
            log::error!(
                "No handlers for device: {}, event dropped!",
                self.device.name()
            );
            return;
        }

        for handler in handlers.iter() {
            handler.handle_event(event);
        }
    }

    /// Submits multiple events in batch.
    pub fn submit_events(&self, events: &[InputEvent]) {
        // Filter out unsupported events
        let supported_events: Vec<_> = events
            .iter()
            .filter(|event| {
                let supported = self.is_event_supported(event);
                if !supported {
                    log::warn!(
                        "Device '{}' does not support event {:?}, dropping event",
                        self.device.name(),
                        event
                    );
                }
                supported
            })
            .cloned()
            .collect();

        if supported_events.is_empty() {
            return;
        }

        let handlers = self.handlers.read();
        if handlers.is_empty() {
            log::error!(
                "No handlers for device: {}, event dropped!",
                self.device.name()
            );
            return;
        }

        for handler in handlers.iter() {
            handler.handle_events(&supported_events);
        }
    }

    /// Gets the underlying device reference.
    pub fn device(&self) -> &Arc<dyn InputDevice> {
        &self.device
    }

    /// Gets the number of connected handlers.
    pub fn handler_count(&self) -> usize {
        self.handlers.read().len()
    }

    /// Checks if the device supports a specific event based on its capabilities.
    fn is_event_supported(&self, event: &InputEvent) -> bool {
        let capability = self.device.capability();

        match event {
            InputEvent::Sync(_) => capability.support_event_type(EventTypes::SYN),
            InputEvent::Key(key_event, _) => {
                capability.support_event_type(EventTypes::KEY) && capability.support_key(*key_event)
            }
            InputEvent::Relative(rel_event, _) => {
                capability.support_event_type(EventTypes::REL)
                    && capability.support_relative_axis(*rel_event)
            }
        }
    }
}

impl InputCapability {
    /// Adds all standard keyboard keys to the capability.
    pub fn add_standard_keyboard_keys(&mut self) {
        let standard_keys = [
            // Function keys
            KeyCode::KeyEsc,
            KeyCode::KeyF1,
            KeyCode::KeyF2,
            KeyCode::KeyF3,
            KeyCode::KeyF4,
            KeyCode::KeyF5,
            KeyCode::KeyF6,
            KeyCode::KeyF7,
            KeyCode::KeyF8,
            KeyCode::KeyF9,
            KeyCode::KeyF10,
            KeyCode::KeyF11,
            KeyCode::KeyF12,
            // Number row
            KeyCode::Key1,
            KeyCode::Key2,
            KeyCode::Key3,
            KeyCode::Key4,
            KeyCode::Key5,
            KeyCode::Key6,
            KeyCode::Key7,
            KeyCode::Key8,
            KeyCode::Key9,
            KeyCode::Key0,
            KeyCode::KeyMinus,
            KeyCode::KeyEqual,
            KeyCode::KeyBackspace,
            // First row (QWERTY)
            KeyCode::KeyTab,
            KeyCode::KeyQ,
            KeyCode::KeyW,
            KeyCode::KeyE,
            KeyCode::KeyR,
            KeyCode::KeyT,
            KeyCode::KeyY,
            KeyCode::KeyU,
            KeyCode::KeyI,
            KeyCode::KeyO,
            KeyCode::KeyP,
            KeyCode::KeyLeftBrace,
            KeyCode::KeyRightBrace,
            KeyCode::KeyBackslash,
            // Second row (ASDF)
            KeyCode::KeyCapsLock,
            KeyCode::KeyA,
            KeyCode::KeyS,
            KeyCode::KeyD,
            KeyCode::KeyF,
            KeyCode::KeyG,
            KeyCode::KeyH,
            KeyCode::KeyJ,
            KeyCode::KeyK,
            KeyCode::KeyL,
            KeyCode::KeySemicolon,
            KeyCode::KeyApostrophe,
            KeyCode::KeyEnter,
            // Third row (ZXCV)
            KeyCode::KeyLeftShift,
            KeyCode::KeyZ,
            KeyCode::KeyX,
            KeyCode::KeyC,
            KeyCode::KeyV,
            KeyCode::KeyB,
            KeyCode::KeyN,
            KeyCode::KeyM,
            KeyCode::KeyComma,
            KeyCode::KeyDot,
            KeyCode::KeySlash,
            KeyCode::KeyRightShift,
            // Bottom row
            KeyCode::KeyLeftCtrl,
            KeyCode::KeyLeftAlt,
            KeyCode::KeySpace,
            KeyCode::KeyRightAlt,
            KeyCode::KeyRightCtrl,
            // Special keys
            KeyCode::KeyGrave,
            KeyCode::KeyLeftMeta,
            KeyCode::KeyRightMeta,
            KeyCode::KeyMenu,
            // Arrow keys
            KeyCode::KeyUp,
            KeyCode::KeyDown,
            KeyCode::KeyLeft,
            KeyCode::KeyRight,
            // Navigation cluster
            KeyCode::KeyHome,
            KeyCode::KeyEnd,
            KeyCode::KeyPageUp,
            KeyCode::KeyPageDown,
            KeyCode::KeyInsert,
            KeyCode::KeyDelete,
            // Lock keys
            KeyCode::KeyNumLock,
            KeyCode::KeyScrollLock,
            // Numpad
            KeyCode::KeyKp0,
            KeyCode::KeyKp1,
            KeyCode::KeyKp2,
            KeyCode::KeyKp3,
            KeyCode::KeyKp4,
            KeyCode::KeyKp5,
            KeyCode::KeyKp6,
            KeyCode::KeyKp7,
            KeyCode::KeyKp8,
            KeyCode::KeyKp9,
            KeyCode::KeyKpDot,
            KeyCode::KeyKpPlus,
            KeyCode::KeyKpMinus,
            KeyCode::KeyKpAsterisk,
            KeyCode::KeyKpSlash,
            KeyCode::KeyKpEnter,
            // Common media keys
            KeyCode::KeyMute,
            KeyCode::KeyVolumeDown,
            KeyCode::KeyVolumeUp,
        ];

        for key in standard_keys {
            self.set_supported_key(key);
        }
    }
}

impl Drop for RegisteredInputDevice {
    fn drop(&mut self) {
        log::info!("Unregistering input device: {}", self.device.name());

        unregister_device(&self.device);
    }
}
