// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use core::{fmt::Debug, mem};

use aster_input::{
    event_type_codes::{EventTypes, KeyEvent, KeyStatus, RelEvent, SynEvent},
    InputCapability, InputDevice as InputDeviceTrait, InputEvent, InputId, RegisteredInputDevice,
};
use aster_util::{field_ptr, safe_ptr::SafePtr};
use bitflags::bitflags;
use log::{debug, info};
use ostd::{
    arch::trap::TrapFrame,
    io::IoMem,
    mm::{DmaDirection, DmaStream, FrameAllocOptions, HasDaddr, PAGE_SIZE},
    sync::SpinLock,
    Pod,
};
use spin::Once;

use super::{InputConfigSelect, VirtioInputConfig, VirtioInputEvent, QUEUE_EVENT, QUEUE_STATUS};
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

/// Global device reference for IRQ handler
static REGISTERED_DEVICE: Once<RegisteredInputDevice> = Once::new();

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

        // Create initial capability and metadata
        let capability = InputCapability::new();

        let mut temp_device = Self {
            config: VirtioInputConfig::new(transport.as_mut()),
            event_queue: SpinLock::new(event_queue),
            status_queue,
            event_table,
            transport: SpinLock::new(transport),
            // Default name, will be updated with actual device name from config
            device_name: "virtio_input".to_string(),
            // Physical path for virtio devices
            device_phys: "virtio/input0".to_string(),
            // Unique identifier (empty for virtio devices)
            device_uniq: "".to_string(),
            // Device ID with virtio-specific values
            // BUS_VIRTUAL (0x06): Virtual bus type
            // vendor (0x0001): Generic vendor ID for standard keyboards
            // product (0x0001): Generic product ID for standard keyboards
            // version (0x0001): Version 1.0
            device_id: InputId::new(InputId::BUS_VIRTUAL, 0x0001, 0x0001, 0x0001),
            capability,
        };

        // Query and update device name from config.
        let name = temp_device.query_config_id_name();
        info!("Virtio input device name: {}", name);
        temp_device.device_name = name;

        // Query and set device capabilities
        temp_device.query_and_set_capabilities();

        let device = Arc::new(temp_device);

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

        let handle_input = {
            let device = device.clone();
            move |_: &TrapFrame| device.handle_irq()
        };
        transport
            .register_queue_callback(QUEUE_EVENT, Box::new(handle_input), false)
            .unwrap();

        transport.finish_init();
        drop(transport);

        // Register with the new input subsystem
        let registered_device = aster_input::register_device(device);
        REGISTERED_DEVICE.call_once(|| registered_device);

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

    fn handle_irq(&self) {
        // Returns true if there may be more events to handle
        let handle_event = |event: &EventBuf| -> bool {
            event.sync().unwrap();
            let virtio_event: VirtioInputEvent = event.read().unwrap();

            match virtio_event.event_type {
                0 => {
                    // EV_SYN events
                    match virtio_event.code {
                        0 => {
                            // SYN_REPORT: end of event sequence, send SynReport
                            if let Some(registered_device) = REGISTERED_DEVICE.get() {
                                let syn_event = InputEvent::sync(SynEvent::SynReport);
                                registered_device.submit_event(&syn_event);
                            }
                            return false; // End of events
                        }
                        _ => {
                            // Other sync events (SYN_DROPPED, SYN_CONFIG, etc.)
                            return true; // Continue processing
                        }
                    }
                }
                // Keyboard events (EV_KEY)
                1 => {
                    let key_status = match virtio_event.value {
                        1 => KeyStatus::Pressed,
                        0 => KeyStatus::Released,
                        _ => return true, // Skip invalid values, continue processing
                    };

                    // Dispatch the key event
                    if let Some(key_code) = map_to_key_event(virtio_event.code) {
                        let key_event = InputEvent::key(key_code, key_status);
                        if let Some(registered_device) = REGISTERED_DEVICE.get() {
                            registered_device.submit_event(&key_event);
                        }
                    } else {
                        debug!(
                            "VirtIO Input: unmapped key code {}, dropped",
                            virtio_event.code
                        );
                    }
                }
                // Relative movement events (EV_REL)
                2 => {
                    if let Some(rel_event) = map_to_rel_event(virtio_event.code) {
                        let rel_value = virtio_event.value as i32;
                        let rel_event = InputEvent::relative(rel_event, rel_value);
                        if let Some(registered_device) = REGISTERED_DEVICE.get() {
                            registered_device.submit_event(&rel_event);
                        }
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
                        "VirtIO Input: Unsupported event type {}, skipping",
                        virtio_event.event_type
                    );
                    return true; // Continue processing other events
                }
            }

            true
        };

        self.pop_pending_events(&handle_event);
    }

    /// Negotiate features for the device specified bits 0~23
    pub(crate) fn negotiate_features(features: u64) -> u64 {
        assert_eq!(features, 0);
        0
    }
}

/// Map key code to KeyEvent enum
fn map_to_key_event(linux_code: u16) -> Option<KeyEvent> {
    Some(match linux_code {
        1 => KeyEvent::KeyEsc,
        2 => KeyEvent::Key1,
        3 => KeyEvent::Key2,
        4 => KeyEvent::Key3,
        5 => KeyEvent::Key4,
        6 => KeyEvent::Key5,
        7 => KeyEvent::Key6,
        8 => KeyEvent::Key7,
        9 => KeyEvent::Key8,
        10 => KeyEvent::Key9,
        11 => KeyEvent::Key0,
        12 => KeyEvent::KeyMinus,
        13 => KeyEvent::KeyEqual,
        14 => KeyEvent::KeyBackspace,
        15 => KeyEvent::KeyTab,
        16 => KeyEvent::KeyQ,
        17 => KeyEvent::KeyW,
        18 => KeyEvent::KeyE,
        19 => KeyEvent::KeyR,
        20 => KeyEvent::KeyT,
        21 => KeyEvent::KeyY,
        22 => KeyEvent::KeyU,
        23 => KeyEvent::KeyI,
        24 => KeyEvent::KeyO,
        25 => KeyEvent::KeyP,
        26 => KeyEvent::KeyLeftBrace,
        27 => KeyEvent::KeyRightBrace,
        28 => KeyEvent::KeyEnter,
        29 => KeyEvent::KeyLeftCtrl,
        30 => KeyEvent::KeyA,
        31 => KeyEvent::KeyS,
        32 => KeyEvent::KeyD,
        33 => KeyEvent::KeyF,
        34 => KeyEvent::KeyG,
        35 => KeyEvent::KeyH,
        36 => KeyEvent::KeyJ,
        37 => KeyEvent::KeyK,
        38 => KeyEvent::KeyL,
        39 => KeyEvent::KeySemicolon,
        40 => KeyEvent::KeyApostrophe,
        41 => KeyEvent::KeyGrave,
        42 => KeyEvent::KeyLeftShift,
        43 => KeyEvent::KeyBackslash,
        44 => KeyEvent::KeyZ,
        45 => KeyEvent::KeyX,
        46 => KeyEvent::KeyC,
        47 => KeyEvent::KeyV,
        48 => KeyEvent::KeyB,
        49 => KeyEvent::KeyN,
        50 => KeyEvent::KeyM,
        51 => KeyEvent::KeyComma,
        52 => KeyEvent::KeyDot,
        53 => KeyEvent::KeySlash,
        54 => KeyEvent::KeyRightShift,
        55 => KeyEvent::KeyKpAsterisk,
        56 => KeyEvent::KeyLeftAlt,
        57 => KeyEvent::KeySpace,
        58 => KeyEvent::KeyCapsLock,
        59 => KeyEvent::KeyF1,
        60 => KeyEvent::KeyF2,
        61 => KeyEvent::KeyF3,
        62 => KeyEvent::KeyF4,
        63 => KeyEvent::KeyF5,
        64 => KeyEvent::KeyF6,
        65 => KeyEvent::KeyF7,
        66 => KeyEvent::KeyF8,
        67 => KeyEvent::KeyF9,
        68 => KeyEvent::KeyF10,
        69 => KeyEvent::KeyNumLock,
        70 => KeyEvent::KeyScrollLock,
        71 => KeyEvent::KeyKp7,
        72 => KeyEvent::KeyKp8,
        73 => KeyEvent::KeyKp9,
        74 => KeyEvent::KeyKpMinus,
        75 => KeyEvent::KeyKp4,
        76 => KeyEvent::KeyKp5,
        77 => KeyEvent::KeyKp6,
        78 => KeyEvent::KeyKpPlus,
        79 => KeyEvent::KeyKp1,
        80 => KeyEvent::KeyKp2,
        81 => KeyEvent::KeyKp3,
        82 => KeyEvent::KeyKp0,
        83 => KeyEvent::KeyKpDot,
        87 => KeyEvent::KeyF11,
        88 => KeyEvent::KeyF12,
        96 => KeyEvent::KeyKpEnter,
        97 => KeyEvent::KeyRightCtrl,
        98 => KeyEvent::KeyKpSlash,
        100 => KeyEvent::KeyRightAlt,
        102 => KeyEvent::KeyHome,
        103 => KeyEvent::KeyUp,
        104 => KeyEvent::KeyPageUp,
        105 => KeyEvent::KeyLeft,
        106 => KeyEvent::KeyRight,
        107 => KeyEvent::KeyEnd,
        108 => KeyEvent::KeyDown,
        109 => KeyEvent::KeyPageDown,
        110 => KeyEvent::KeyInsert,
        111 => KeyEvent::KeyDelete,
        113 => KeyEvent::KeyMute,
        114 => KeyEvent::KeyVolumeDown,
        115 => KeyEvent::KeyVolumeUp,
        125 => KeyEvent::KeyLeftMeta,
        126 => KeyEvent::KeyRightMeta,
        139 => KeyEvent::KeyMenu,
        _ => return None,
    })
}

/// Map relative axis code to RelEvent enum
fn map_to_rel_event(linux_code: u16) -> Option<RelEvent> {
    Some(match linux_code {
        0x00 => RelEvent::RelX,
        0x01 => RelEvent::RelY,
        0x02 => RelEvent::RelZ,
        0x03 => RelEvent::RelRx,
        0x04 => RelEvent::RelRy,
        0x05 => RelEvent::RelRz,
        0x06 => RelEvent::RelHWheel,
        0x07 => RelEvent::RelDial,
        0x08 => RelEvent::RelWheel,
        0x09 => RelEvent::RelMisc,
        0x0a => RelEvent::RelReserved,
        0x0b => RelEvent::RelWheelHiRes,
        0x0c => RelEvent::RelHWheelHiRes,
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
    /// Query device capabilities from VirtIO config space and set them
    fn query_and_set_capabilities(&mut self) {
        // Query supported event types
        let ev_syn = self.query_ev_bits(EventTypes::SYN.as_u16());
        let ev_key = self.query_ev_bits(EventTypes::KEY.as_u16());
        let ev_rel = self.query_ev_bits(EventTypes::REL.as_u16());

        let capability = &mut self.capability;

        // Set event type capabilities
        if ev_syn.is_some() {
            capability.set_supported_event_type(EventTypes::SYN);
        }
        if ev_key.is_some() {
            capability.set_supported_event_type(EventTypes::KEY);
        }
        if ev_rel.is_some() {
            capability.set_supported_event_type(EventTypes::REL);
        }

        // Query and set key capabilities
        if let Some(key_bits) = &ev_key {
            for bit in 0..key_bits.len() * 8 {
                if key_bits[bit / 8] & (1 << (bit % 8)) != 0 {
                    if let Some(key_event) = map_to_key_event(bit as u16) {
                        capability.set_supported_key(key_event);
                    }
                }
            }
        }

        // Query and set relative axis capabilities
        if let Some(rel_bits) = &ev_rel {
            for bit in 0..rel_bits.len() * 8 {
                if rel_bits[bit / 8] & (1 << (bit % 8)) != 0 {
                    if let Some(rel_event) = map_to_rel_event(bit as u16) {
                        capability.set_supported_relative_axis(rel_event);
                    }
                }
            }
        }

        info!(
            "VirtIO input device capabilities set: SYN={}, KEY={}, REL={}",
            ev_syn.is_some(),
            ev_key.is_some(),
            ev_rel.is_some()
        );
    }

    /// Query event bits for a specific event type
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
    stream: DmaStream,
    num_events: usize,
}

impl EventTable {
    fn new(num_events: usize) -> Self {
        assert!(num_events * mem::size_of::<VirtioInputEvent>() <= PAGE_SIZE);

        let segment = FrameAllocOptions::new()
            .zeroed(true)
            .alloc_segment(1)
            .unwrap();
        debug_assert!(VirtioInputEvent::default()
            .as_bytes()
            .iter()
            .all(|b| *b == 0));

        let stream = DmaStream::map(segment.into(), DmaDirection::FromDevice, false).unwrap();
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

const EVENT_SIZE: usize = core::mem::size_of::<VirtioInputEvent>();
type EventBuf<'a> = SafePtr<VirtioInputEvent, &'a DmaStream>;

impl<T, M: HasDaddr> DmaBuf for SafePtr<T, M> {
    fn len(&self) -> usize {
        core::mem::size_of::<T>()
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
