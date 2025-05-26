// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use core::{fmt::Debug, mem};

use aster_input::{
    event_type_codes::KeyStatus,
    InputEvent,
};
use aster_util::{field_ptr, safe_ptr::SafePtr};
use aster_input::{InputDevice as AsterInputDevice, InputDeviceMeta, InputID};
use alloc::vec;
use aster_input::event_type_codes::*;

use bitflags::bitflags;
use log::{debug, info};
use ostd::{
    arch::trap::TrapFrame,
    io::IoMem,
    mm::{DmaDirection, DmaStream, FrameAllocOptions, HasDaddr, PAGE_SIZE},
    sync::{LocalIrqDisabled, RwLock, SpinLock},
    Pod,
};

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
    #[expect(clippy::type_complexity)]
    callbacks: RwLock<Vec<Arc<dyn Fn(InputEvent) + Send + Sync + 'static>>, LocalIrqDisabled>,
    transport: SpinLock<Box<dyn VirtioTransport>>,
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

        let device = Arc::new(Self {
            config: VirtioInputConfig::new(transport.as_mut()),
            event_queue: SpinLock::new(event_queue),
            status_queue,
            event_table,
            transport: SpinLock::new(transport),
            callbacks: RwLock::new(Vec::new()),
        });

        let name = device.query_config_id_name();
        info!("Virtio input device name:{}", name);

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

        aster_input::register_device(super::DEVICE_NAME.to_string(), device);

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
        let callbacks = self.callbacks.read();
        // Returns true if there may be more events to handle
        let handle_event = |event: &EventBuf| -> bool {
            event.sync().unwrap();
            let event: VirtioInputEvent = event.read().unwrap();

            match event.event_type {
                0 => return false,
                // Keyboard
                1 => {}
                // TODO: Support mouse device.
                _ => return true,
            }

            let status = match event.value {
                1 => KeyStatus::Pressed,
                0 => KeyStatus::Released,
                _ => return false,
            };

            let event = InputEvent {
                time: 0, // Replace with actual timestamp if available
                type_: 0x01, // EV_KEY
                code: event.code,
                value: match status {
                    KeyStatus::Pressed => 1,
                    KeyStatus::Released => 0,
                },
            };
    
            info!("Input Event:{:?}", event);

            for callback in callbacks.iter() {
                callback(event);
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

impl AsterInputDevice for InputDevice {
    fn metadata(&self) -> InputDeviceMeta {
        let id = InputID {
            bustype: 0x11,
            vendor_id: 0x1234,   
            product_id: 0x5678,  
            version: 1,       
        };
        InputDeviceMeta {
            name: self.query_config_id_name(),
            phys: "dev/virtio".to_string(),
            uniq: "NULL".to_string(),
            version: 65537,
            id: id,
        }
    }

    fn get_prop_bit(&self) -> Vec<PropType> {
        vec![]
    }

    fn get_ev_bit(&self) -> Vec<EventType> {
        vec![]
    }

    fn get_key_bit(&self) -> Vec<KeyEvent> {
        vec![]
    }

    fn get_led_bit(&self) -> Vec<LedEvent> {
        vec![]
    }

    fn get_msc_bit(&self) -> Vec<MiscEvent> {
        vec![]
    }

    fn get_rel_bit(&self) -> Vec<RelEvent> {
        vec![]
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
