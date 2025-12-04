// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use core::fmt::Debug;

use aster_input::{
    event_type_codes::{EventTypes, KeyCode, KeyStatus, RelCode, SynEvent},
    input_dev::{
        InputCapability, InputDevice as InputDeviceTrait, InputEvent, InputId,
        RegisteredInputDevice,
    },
};
use aster_util::{field_ptr, safe_ptr::SafePtr};
use bitflags::bitflags;
use log::{debug, info};
use ostd::{
    Pod,
    arch::trap::TrapFrame,
    io::IoMem,
    mm::{FrameAllocOptions, HasDaddr, PAGE_SIZE, dma::DmaStream},
    sync::SpinLock,
};

use super::{InputConfigSelect, QUEUE_EVENT, QUEUE_STATUS, VirtioInputConfig, VirtioInputEvent};
use crate::{
    device::VirtioDeviceError, dma_buf::DmaBuf, queue::VirtQueue, transport::VirtioTransport,
};

bitflags! {
    /// The properties of input device.
    ///
    /// Ref: Linux input-event-codes.h
    pub struct InputProp : u8 {
        /// Needs a pointer
        const POINTER           = 1 << 0;
        /// Direct input devices
        const DIRECT            = 1 << 1;
        /// Has button(s) under pad
        const BUTTONPAD         = 1 << 2;
        /// Touch rectangle only
        const SEMI_MT           = 1 << 3;
        /// Softbuttons at top of pad
        const TOPBUTTONPAD      = 1 << 4;
        /// Is a pointing stick
        const POINTING_STICK    = 1 << 5;
        /// Has accelerometer
        const ACCELEROMETER     = 1 << 6;
    }
}

pub const SYN: u8 = 0x00;
pub const KEY: u8 = 0x01;
pub const REL: u8 = 0x02;
pub const ABS: u8 = 0x03;
pub const MSC: u8 = 0x04;
pub const SW: u8 = 0x05;
pub const LED: u8 = 0x11;
pub const SND: u8 = 0x12;
pub const REP: u8 = 0x14;
pub const FF: u8 = 0x15;
pub const PWR: u8 = 0x16;
pub const FF_STATUS: u8 = 0x17;

const QUEUE_SIZE: u16 = 64;

/// Virtual human interface devices such as keyboards, mice and tablets.
///
/// An instance of the virtio device represents one such input device.
/// Device behavior mirrors that of the evdev layer in Linux,
/// making pass-through implementations on top of evdev easy.
pub struct InputDevice {
    config: SafePtr<VirtioInputConfig, IoMem>,
    event_queue: SpinLock<VirtQueue>,
    status_queue: VirtQueue,
    event_table: EventTable,
    transport: SpinLock<Box<dyn VirtioTransport>>,
    device_name: String,
    device_phys: String,
    device_uniq: String,
    device_id: InputId,
    capability: InputCapability,
}

impl InputDevice {
    /// Create a new VirtIO-Input driver.
    /// msix_vector_left should at least have one element or n elements where n is the virtqueue amount
    pub fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let mut event_queue = VirtQueue::new(QUEUE_EVENT, QUEUE_SIZE, transport.as_mut())
            .expect("create event virtqueue failed");
        let status_queue = VirtQueue::new(QUEUE_STATUS, QUEUE_SIZE, transport.as_mut())
            .expect("create status virtqueue failed");

        let event_table = EventTable::new(QUEUE_SIZE as usize);
        for i in 0..event_table.num_events() {
            let event_buf = event_table.get(i);
            let token = event_queue.add_dma_buf(&[], &[&event_buf]);
            match token {
                Ok(value) => {
                    assert_eq!(value, i as u16);
                }
                Err(_) => {
                    return Err(VirtioDeviceError::QueueUnknownError);
                }
            }
        }

        let device = {
            let mut device = Self {
                config: VirtioInputConfig::new(transport.as_mut()),
                event_queue: SpinLock::new(event_queue),
                status_queue,
                event_table,
                transport: SpinLock::new(transport),
                // Default name, will be updated with actual device name from config.
                device_name: "virtio_input".to_string(),
                // Physical path for virtio devices.
                device_phys: "virtio/input0".to_string(),
                // Unique identifier (empty for virtio devices).
                device_uniq: "".to_string(),
                // Device ID with virtio-specific values.
                // BUS_VIRTUAL (0x06): Virtual bus type
                // vendor (0x0001): Generic vendor ID for standard keyboards
                // product (0x0001): Generic product ID for standard keyboards
                // version (0x0001): Version 1.0
                device_id: InputId::new(InputId::BUS_VIRTUAL, 0x0001, 0x0001, 0x0001),
                capability: InputCapability::new(),
            };

            // Query and update device name from config.
            let name = device.query_config_id_name();
            info!("Virtio input device name: {}", name);
            device.device_name = name;

            // Query and set device capabilities.
            device.query_and_set_capabilities();

            Arc::new(device)
        };

        let input_prop = device.query_config_prop_bits();
        if let Some(prop) = input_prop {
            debug!("input device prop: {:?}", prop);
        } else {
            debug!("input device has no properties or the properties is not defined");
        }

        let mut transport = device.transport.disable_irq().lock();
        fn config_space_change(_: &TrapFrame) {
            debug!("input device config space change");
        }
        transport
            .register_cfg_callback(Box::new(config_space_change))
            .unwrap();
        transport.finish_init();
        drop(transport);

        // Register with the input subsystem.
        let registered_device = aster_input::register_device(device.clone());

        let mut transport = device.transport.disable_irq().lock();
        let handle_input = {
            let device = device.clone();
            move |_: &TrapFrame| device.handle_irq(&registered_device)
        };
        transport
            .register_queue_callback(QUEUE_EVENT, Box::new(handle_input), false)
            .unwrap();
        drop(transport);

        Ok(())
    }

    /// Pop the pending event.
    fn pop_pending_events(&self, handle_event: &impl Fn(&EventBuf) -> bool) {
        let mut event_queue = self.event_queue.disable_irq().lock();

        // one interrupt may contain several input events, so it should loop
        while let Ok((token, _)) = event_queue.pop_used() {
            debug_assert!(token < QUEUE_SIZE);
            let ptr = self.event_table.get(token as usize);
            let res = handle_event(&ptr);
            let new_token = event_queue.add_dma_buf(&[], &[&ptr]).unwrap();
            // This only works because nothing happen between `pop_used` and `add` that affects
            // the list of free descriptors in the queue, so `add` reuses the descriptor which
            // was just freed by `pop_used`.
            assert_eq!(new_token, token);

            if !res {
                break;
            }
        }
    }

    pub fn query_config_id_name(&self) -> String {
        let size = self.select_config(InputConfigSelect::IdName, 0);

        let out = {
            // TODO: Add a general API to read this byte-by-byte.
            let mut out = Vec::with_capacity(size);
            let mut data_ptr = field_ptr!(&self.config, VirtioInputConfig, data).cast::<u8>();
            for _ in 0..size {
                out.push(data_ptr.read_once().unwrap());
                data_ptr.byte_add(1);
            }
            out
        };

        String::from_utf8(out).unwrap()
    }

    pub fn query_config_prop_bits(&self) -> Option<InputProp> {
        let size = self.select_config(InputConfigSelect::PropBits, 0);
        if size == 0 {
            return None;
        }
        assert!(size == 1);

        let data_ptr = field_ptr!(&self.config, VirtioInputConfig, data);
        InputProp::from_bits(data_ptr.cast::<u8>().read_once().unwrap())
    }

    /// Query a specific piece of information by `select` and `subsel`, return the result size.
    fn select_config(&self, select: InputConfigSelect, subsel: u8) -> usize {
        field_ptr!(&self.config, VirtioInputConfig, select)
            .write_once(&(select as u8))
            .unwrap();
        field_ptr!(&self.config, VirtioInputConfig, subsel)
            .write_once(&subsel)
            .unwrap();
        field_ptr!(&self.config, VirtioInputConfig, size)
            .read_once()
            .unwrap() as usize
    }

    fn handle_irq(&self, registered_device: &RegisteredInputDevice) {
        self.pop_pending_events(&|event: &EventBuf| Self::handle_event(event, registered_device));
    }

    /// Handles an event and returns whether there may be more event to handle.
    fn handle_event(event: &EventBuf, registered_device: &RegisteredInputDevice) -> bool {
        let virtio_event: VirtioInputEvent = event.read().unwrap();

        match virtio_event.event_type {
            // EV_SYN events
            0 => match virtio_event.code {
                0 => {
                    // SYN_REPORT: end of event sequence, send `SynEvent::Report`
                    let syn_event = InputEvent::from_sync_event(SynEvent::Report);
                    registered_device.submit_events(&[syn_event]);
                    return false;
                }
                _ => {
                    // Other sync events (SYN_DROPPED, SYN_CONFIG, etc.)
                    return true;
                }
            },
            // Keyboard events (EV_KEY)
            1 => {
                let key_status = match virtio_event.value {
                    1 => KeyStatus::Pressed,
                    0 => KeyStatus::Released,
                    _ => return true,
                };

                if let Some(key_code) = map_to_key_code(virtio_event.code) {
                    let key_event = InputEvent::from_key_and_status(key_code, key_status);
                    registered_device.submit_events(&[key_event]);
                } else {
                    debug!(
                        "VirtIO Input: unmapped key code {}, dropped",
                        virtio_event.code
                    );
                }
            }
            // Relative movement events (EV_REL)
            2 => {
                if let Some(rel_code) = map_to_rel_code(virtio_event.code) {
                    let rel_value = virtio_event.value as i32;
                    let rel_event = InputEvent::from_relative_move(rel_code, rel_value);
                    registered_device.submit_events(&[rel_event]);
                } else {
                    debug!(
                        "VirtIO Input: unmapped relative event code {}, dropped",
                        virtio_event.code
                    );
                }
            }

            // Other event types
            _ => {
                debug!(
                    "VirtIO Input: unsupported event type {}, skipping",
                    virtio_event.event_type
                );
                return true;
            }
        }

        true
    }

    /// Negotiates features for the device specified bits 0~23.
    pub(crate) fn negotiate_features(features: u64) -> u64 {
        assert_eq!(features, 0);
        0
    }
}

/// Maps a VirtIO key code to a [`KeyCode`].
fn map_to_key_code(virtio_code: u16) -> Option<KeyCode> {
    Some(match virtio_code {
        1 => KeyCode::Esc,
        2 => KeyCode::Num1,
        3 => KeyCode::Num2,
        4 => KeyCode::Num3,
        5 => KeyCode::Num4,
        6 => KeyCode::Num5,
        7 => KeyCode::Num6,
        8 => KeyCode::Num7,
        9 => KeyCode::Num8,
        10 => KeyCode::Num9,
        11 => KeyCode::Num0,
        12 => KeyCode::Minus,
        13 => KeyCode::Equal,
        14 => KeyCode::Backspace,
        15 => KeyCode::Tab,
        16 => KeyCode::Q,
        17 => KeyCode::W,
        18 => KeyCode::E,
        19 => KeyCode::R,
        20 => KeyCode::T,
        21 => KeyCode::Y,
        22 => KeyCode::U,
        23 => KeyCode::I,
        24 => KeyCode::O,
        25 => KeyCode::P,
        26 => KeyCode::LeftBrace,
        27 => KeyCode::RightBrace,
        28 => KeyCode::Enter,
        29 => KeyCode::LeftCtrl,
        30 => KeyCode::A,
        31 => KeyCode::S,
        32 => KeyCode::D,
        33 => KeyCode::F,
        34 => KeyCode::G,
        35 => KeyCode::H,
        36 => KeyCode::J,
        37 => KeyCode::K,
        38 => KeyCode::L,
        39 => KeyCode::Semicolon,
        40 => KeyCode::Apostrophe,
        41 => KeyCode::Grave,
        42 => KeyCode::LeftShift,
        43 => KeyCode::Backslash,
        44 => KeyCode::Z,
        45 => KeyCode::X,
        46 => KeyCode::C,
        47 => KeyCode::V,
        48 => KeyCode::B,
        49 => KeyCode::N,
        50 => KeyCode::M,
        51 => KeyCode::Comma,
        52 => KeyCode::Dot,
        53 => KeyCode::Slash,
        54 => KeyCode::RightShift,
        55 => KeyCode::KpAsterisk,
        56 => KeyCode::LeftAlt,
        57 => KeyCode::Space,
        58 => KeyCode::CapsLock,
        59 => KeyCode::F1,
        60 => KeyCode::F2,
        61 => KeyCode::F3,
        62 => KeyCode::F4,
        63 => KeyCode::F5,
        64 => KeyCode::F6,
        65 => KeyCode::F7,
        66 => KeyCode::F8,
        67 => KeyCode::F9,
        68 => KeyCode::F10,
        69 => KeyCode::NumLock,
        70 => KeyCode::ScrollLock,
        71 => KeyCode::Kp7,
        72 => KeyCode::Kp8,
        73 => KeyCode::Kp9,
        74 => KeyCode::KpMinus,
        75 => KeyCode::Kp4,
        76 => KeyCode::Kp5,
        77 => KeyCode::Kp6,
        78 => KeyCode::KpPlus,
        79 => KeyCode::Kp1,
        80 => KeyCode::Kp2,
        81 => KeyCode::Kp3,
        82 => KeyCode::Kp0,
        83 => KeyCode::KpDot,
        87 => KeyCode::F11,
        88 => KeyCode::F12,
        96 => KeyCode::KpEnter,
        97 => KeyCode::RightCtrl,
        98 => KeyCode::KpSlash,
        100 => KeyCode::RightAlt,
        102 => KeyCode::Home,
        103 => KeyCode::Up,
        104 => KeyCode::PageUp,
        105 => KeyCode::Left,
        106 => KeyCode::Right,
        107 => KeyCode::End,
        108 => KeyCode::Down,
        109 => KeyCode::PageDown,
        110 => KeyCode::Insert,
        111 => KeyCode::Delete,
        113 => KeyCode::Mute,
        114 => KeyCode::VolumeDown,
        115 => KeyCode::VolumeUp,
        125 => KeyCode::LeftMeta,
        126 => KeyCode::RightMeta,
        139 => KeyCode::Menu,
        _ => return None,
    })
}

/// Maps a VirtIO relative axis code to a [`RelCode`].
fn map_to_rel_code(virtio_code: u16) -> Option<RelCode> {
    Some(match virtio_code {
        0x00 => RelCode::X,
        0x01 => RelCode::Y,
        0x02 => RelCode::Z,
        0x03 => RelCode::Rx,
        0x04 => RelCode::Ry,
        0x05 => RelCode::Rz,
        0x06 => RelCode::HWheel,
        0x07 => RelCode::Dial,
        0x08 => RelCode::Wheel,
        0x09 => RelCode::Misc,
        0x0a => RelCode::Reserved,
        0x0b => RelCode::WheelHiRes,
        0x0c => RelCode::HWheelHiRes,
        _ => return None,
    })
}

impl InputDeviceTrait for InputDevice {
    fn name(&self) -> &str {
        &self.device_name
    }

    fn phys(&self) -> &str {
        &self.device_phys
    }

    fn uniq(&self) -> &str {
        &self.device_uniq
    }

    fn id(&self) -> InputId {
        self.device_id
    }

    fn capability(&self) -> &InputCapability {
        &self.capability
    }
}

impl InputDevice {
    /// Queries device capabilities from VirtIO config space and sets them in `self`.
    fn query_and_set_capabilities(&mut self) {
        // Query supported event types.
        let ev_key = self.query_ev_bits(EventTypes::KEY.as_index());
        let ev_rel = self.query_ev_bits(EventTypes::REL.as_index());

        let capability = &mut self.capability;
        capability.set_supported_event_type(EventTypes::SYN);

        if ev_key.is_some() {
            capability.set_supported_event_type(EventTypes::KEY);
        }
        if ev_rel.is_some() {
            capability.set_supported_event_type(EventTypes::REL);
        }

        // Set key capabilities.
        if let Some(key_bits) = &ev_key {
            for bit in 0..key_bits.len() * 8 {
                if key_bits[bit / 8] & (1 << (bit % 8)) != 0
                    && let Some(key_code) = map_to_key_code(bit as u16)
                {
                    capability.set_supported_key(key_code);
                }
            }
        }

        // Set relative axis capabilities.
        if let Some(rel_bits) = &ev_rel {
            for bit in 0..rel_bits.len() * 8 {
                if rel_bits[bit / 8] & (1 << (bit % 8)) != 0
                    && let Some(rel_code) = map_to_rel_code(bit as u16)
                {
                    capability.set_supported_relative_axis(rel_code);
                }
            }
        }

        info!(
            "VirtIO input device capabilities set: KEY={}, REL={}",
            ev_key.is_some(),
            ev_rel.is_some()
        );
    }

    /// Queries event bits for a specific event type.
    fn query_ev_bits(&self, event_type: u16) -> Option<Vec<u8>> {
        let size = self.select_config(InputConfigSelect::EvBits, event_type as u8);
        if size == 0 {
            return None;
        }

        let mut bits = Vec::with_capacity(size);
        let data_ptr = field_ptr!(&self.config, VirtioInputConfig, data).cast::<u8>();
        for i in 0..size {
            let mut ptr = data_ptr.clone();
            ptr.byte_add(i);
            bits.push(ptr.read_once().unwrap());
        }
        Some(bits)
    }
}

/// A event table consists of many event buffers,
/// each of which is large enough to contain a `VirtioInputEvent`.
#[derive(Debug)]
struct EventTable {
    stream: Arc<DmaStream>,
    num_events: usize,
}

impl EventTable {
    fn new(num_events: usize) -> Self {
        assert!(num_events * size_of::<VirtioInputEvent>() <= PAGE_SIZE);

        let segment = FrameAllocOptions::new()
            .zeroed(true)
            .alloc_segment(1)
            .unwrap();
        debug_assert!(
            VirtioInputEvent::default()
                .as_bytes()
                .iter()
                .all(|b| *b == 0)
        );

        let stream = Arc::new(DmaStream::map(segment.into(), false).unwrap());
        Self { stream, num_events }
    }

    fn get(&self, idx: usize) -> EventBuf<'_> {
        assert!(idx < self.num_events);

        let offset = idx * EVENT_SIZE;
        SafePtr::new(&self.stream, offset)
    }

    const fn num_events(&self) -> usize {
        self.num_events
    }
}

const EVENT_SIZE: usize = size_of::<VirtioInputEvent>();
type EventBuf<'a> = SafePtr<VirtioInputEvent, &'a DmaStream>;

impl<T, M: HasDaddr> DmaBuf for SafePtr<T, M> {
    fn len(&self) -> usize {
        size_of::<T>()
    }
}

impl Debug for InputDevice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InputDevice")
            .field("config", &self.config)
            .field("event_queue", &self.event_queue)
            .field("status_queue", &self.status_queue)
            .field("event_buf", &self.event_table)
            .field("transport", &self.transport)
            .finish()
    }
}
