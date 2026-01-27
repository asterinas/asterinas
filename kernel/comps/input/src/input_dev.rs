// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};
use core::{any::Any, fmt::Debug};

use ostd::sync::{RwLock, WriteIrqDisabled};

use crate::{
    event_type_codes::{EventTypes, KeyCode, KeyCodeSet, KeyStatus, RelCode, RelCodeSet, SynEvent},
    input_handler::BoundInputHandler,
    unregister_device,
};

/// Input event.
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
    pub fn from_sync_event(sync_type: SynEvent) -> Self {
        Self::Sync(sync_type)
    }

    /// Creates a key event.
    pub fn from_key_and_status(key: KeyCode, status: KeyStatus) -> Self {
        Self::Key(key, status)
    }

    /// Creates a relative movement event.
    pub fn from_relative_move(axis: RelCode, value: i32) -> Self {
        Self::Relative(axis, value)
    }

    /// Converts enum to raw Linux input event triplet (type, code, value).
    pub fn to_raw(&self) -> (u16, u16, i32) {
        match self {
            InputEvent::Sync(sync_type) => (
                EventTypes::SYN.as_index(),
                *sync_type as u16,
                0, // Sync events always have value = 0
            ),
            InputEvent::Key(key, status) => {
                (EventTypes::KEY.as_index(), *key as u16, *status as i32)
            }
            InputEvent::Relative(axis, value) => (EventTypes::REL.as_index(), *axis as u16, *value),
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

/// Input device identifier.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct InputId {
    /// Bus type identifier.
    bustype: u16,
    /// Vendor ID of the device manufacturer.
    vendor: u16,
    /// Product ID of the specific device model.
    product: u16,
    /// Version number of the device.
    version: u16,
}

impl InputId {
    /// Creates a new `InputId` with the specified values.
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

    // Common bus types.
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
    supported_keys: KeyCodeSet,
    /// Supported relative axis codes.
    supported_relative_axes: RelCodeSet,
    // TODO: Add supported_absolute_axes, supported_misc, etc.
}

impl Default for InputCapability {
    fn default() -> Self {
        Self::new()
    }
}

impl InputCapability {
    /// Creates a new empty input capability.
    pub fn new() -> Self {
        Self {
            supported_event_types: EventTypes::new(),
            supported_keys: KeyCodeSet::new(),
            supported_relative_axes: RelCodeSet::new(),
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
        self.supported_keys.contain(key_code)
    }

    /// Detects whether the device is keyboard-like.
    ///
    /// We follow the rules defined by Linux: a device is a keyboard if
    /// - it supports `EV_KEY`, and
    /// - it has any non-button key set (i.e., below `BTN_MISC`).
    pub fn look_like_keyboard(&self) -> bool {
        if !self.support_event_type(EventTypes::KEY) {
            return false;
        }

        self.supported_keys
            .contain_any(0..KeyCode::BtnMisc as usize)
    }

    /// Removes support for a key code.
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
        self.supported_relative_axes.contain(rel_code)
    }

    /// Removes support for a relative code.
    pub fn clear_supported_relative_axis(&mut self, rel_code: RelCode) {
        self.supported_relative_axes.clear(rel_code);
    }

    /// Returns the supported event types as a bitmap.
    pub fn event_types_bits(&self) -> u32 {
        self.supported_event_types.bits()
    }

    /// Returns the supported key code bitmap as bytes.
    pub fn supported_keys_bitmap(&self) -> &[u8] {
        self.supported_keys.as_raw_slice()
    }

    /// Returns the supported relative axes bitmap as bytes.
    pub fn supported_relative_axes_bitmap(&self) -> &[u8] {
        self.supported_relative_axes.as_raw_slice()
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
    /// Reference to bound handlers for direct event dispatch.
    handlers: Arc<RwLock<Vec<BoundInputHandler>, WriteIrqDisabled>>,
}

impl RegisteredInputDevice {
    pub(crate) fn new(
        device: Arc<dyn InputDevice>,
        handlers: Arc<RwLock<Vec<BoundInputHandler>, WriteIrqDisabled>>,
    ) -> Self {
        Self { device, handlers }
    }

    /// Submits multiple events in batch.
    pub fn submit_events(&self, events: &[InputEvent]) {
        debug_assert!(
            events.iter().all(|e| self.is_event_supported(e)),
            "Device '{}' submitted unsupported event",
            self.device.name()
        );

        let handlers = self.handlers.read();
        if handlers.is_empty() {
            log::debug!(
                "Input: dropped events from device {} because it has no handlers",
                self.device.name()
            );
            return;
        }

        for bound_handler in handlers.iter() {
            bound_handler.handler.handle_events(events);
        }
    }

    /// Gets the underlying device reference.
    pub fn device(&self) -> &Arc<dyn InputDevice> {
        &self.device
    }

    /// Counts the number of connected handlers.
    pub fn count_handlers(&self) -> usize {
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
            KeyCode::Esc,
            KeyCode::F1,
            KeyCode::F2,
            KeyCode::F3,
            KeyCode::F4,
            KeyCode::F5,
            KeyCode::F6,
            KeyCode::F7,
            KeyCode::F8,
            KeyCode::F9,
            KeyCode::F10,
            KeyCode::F11,
            KeyCode::F12,
            // Number row
            KeyCode::Num1,
            KeyCode::Num2,
            KeyCode::Num3,
            KeyCode::Num4,
            KeyCode::Num5,
            KeyCode::Num6,
            KeyCode::Num7,
            KeyCode::Num8,
            KeyCode::Num9,
            KeyCode::Num0,
            KeyCode::Minus,
            KeyCode::Equal,
            KeyCode::Backspace,
            // First row (QWERTY)
            KeyCode::Tab,
            KeyCode::Q,
            KeyCode::W,
            KeyCode::E,
            KeyCode::R,
            KeyCode::T,
            KeyCode::Y,
            KeyCode::U,
            KeyCode::I,
            KeyCode::O,
            KeyCode::P,
            KeyCode::LeftBrace,
            KeyCode::RightBrace,
            KeyCode::Backslash,
            // Second row (ASDF)
            KeyCode::CapsLock,
            KeyCode::A,
            KeyCode::S,
            KeyCode::D,
            KeyCode::F,
            KeyCode::G,
            KeyCode::H,
            KeyCode::J,
            KeyCode::K,
            KeyCode::L,
            KeyCode::Semicolon,
            KeyCode::Apostrophe,
            KeyCode::Enter,
            // Third row (ZXCV)
            KeyCode::LeftShift,
            KeyCode::Z,
            KeyCode::X,
            KeyCode::C,
            KeyCode::V,
            KeyCode::B,
            KeyCode::N,
            KeyCode::M,
            KeyCode::Comma,
            KeyCode::Dot,
            KeyCode::Slash,
            KeyCode::RightShift,
            // Bottom row
            KeyCode::LeftCtrl,
            KeyCode::LeftAlt,
            KeyCode::Space,
            KeyCode::RightAlt,
            KeyCode::RightCtrl,
            // Special keys
            KeyCode::Grave,
            KeyCode::LeftMeta,
            KeyCode::RightMeta,
            KeyCode::Menu,
            // Arrow keys
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Left,
            KeyCode::Right,
            // Navigation cluster
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Insert,
            KeyCode::Delete,
            // Lock keys
            KeyCode::NumLock,
            KeyCode::ScrollLock,
            // Numpad
            KeyCode::Kp0,
            KeyCode::Kp1,
            KeyCode::Kp2,
            KeyCode::Kp3,
            KeyCode::Kp4,
            KeyCode::Kp5,
            KeyCode::Kp6,
            KeyCode::Kp7,
            KeyCode::Kp8,
            KeyCode::Kp9,
            KeyCode::KpDot,
            KeyCode::KpPlus,
            KeyCode::KpMinus,
            KeyCode::KpAsterisk,
            KeyCode::KpSlash,
            KeyCode::KpEnter,
            // Common media keys
            KeyCode::Mute,
            KeyCode::VolumeDown,
            KeyCode::VolumeUp,
        ];

        for key in standard_keys {
            self.set_supported_key(key);
        }
    }
}

impl Drop for RegisteredInputDevice {
    fn drop(&mut self) {
        unregister_device(&self.device).unwrap();
    }
}
