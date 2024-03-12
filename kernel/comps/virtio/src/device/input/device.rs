// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use core::fmt::Debug;

use aster_frame::{
    io_mem::IoMem,
    offset_of,
    sync::SpinLock,
    trap::TrapFrame,
    vm::{Daddr, DmaDirection, DmaStream, HasDaddr, VmAllocOptions, VmIo, VmReader},
};
use aster_input::{
    key::{Key, KeyStatus},
    InputEvent,
};
use aster_util::{field_ptr, safe_ptr::SafePtr};
use bitflags::bitflags;
use log::{debug, info};

use super::{InputConfigSelect, VirtioInputConfig, VirtioInputEvent, QUEUE_EVENT, QUEUE_STATUS};
use crate::{device::VirtioDeviceError, queue::VirtQueue, transport::VirtioTransport};

bitflags! {
    /// The properties of input device.
    ///
    /// Ref: Linux input-event-codes.h
    pub struct InputProp : u8{
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
    event_buf: EventBuf,
    #[allow(clippy::type_complexity)]
    callbacks: SpinLock<Vec<Arc<dyn Fn(InputEvent) + Send + Sync + 'static>>>,
    transport: Box<dyn VirtioTransport>,
}

impl InputDevice {
    /// Create a new VirtIO-Input driver.
    /// msix_vector_left should at least have one element or n elements where n is the virtqueue amount
    pub fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let mut event_queue = VirtQueue::new(QUEUE_EVENT, QUEUE_SIZE, transport.as_mut())
            .expect("create event virtqueue failed");
        let status_queue = VirtQueue::new(QUEUE_STATUS, QUEUE_SIZE, transport.as_mut())
            .expect("create status virtqueue failed");

        let event_buf = EventBuf::new();
        for (i, event) in event_buf.iter().enumerate() {
            let token = event_queue.add_dma_buf(&[], &[&(event.daddr(), event.size())]);
            match token {
                Ok(value) => {
                    assert_eq!(value, i as u16);
                }
                Err(_) => {
                    return Err(VirtioDeviceError::QueueUnknownError);
                }
            }
        }

        let mut device = Self {
            config: VirtioInputConfig::new(transport.as_mut()),
            event_queue: SpinLock::new(event_queue),
            status_queue,
            event_buf,
            transport,
            callbacks: SpinLock::new(Vec::new()),
        };

        let mut raw_name: [u8; 128] = [0; 128];
        device.query_config_select(InputConfigSelect::IdName, 0, &mut raw_name);
        let name = String::from_utf8(raw_name.to_vec()).unwrap();
        info!("Virtio input device name:{}", name);

        let mut prop: [u8; 128] = [0; 128];
        device.query_config_select(InputConfigSelect::PropBits, 0, &mut prop);
        let input_prop = InputProp::from_bits(prop[0]).unwrap();
        debug!("input device prop:{:?}", input_prop);

        fn handle_input(_: &TrapFrame) {
            debug!("Handle Virtio input interrupt");
            let device = aster_input::get_device(super::DEVICE_NAME).unwrap();
            device.handle_irq();
        }

        fn config_space_change(_: &TrapFrame) {
            debug!("input device config space change");
        }

        device
            .transport
            .register_cfg_callback(Box::new(config_space_change))
            .unwrap();
        device
            .transport
            .register_queue_callback(QUEUE_EVENT, Box::new(handle_input), false)
            .unwrap();

        device.transport.finish_init();

        aster_input::register_device(super::DEVICE_NAME.to_string(), Arc::new(device));

        Ok(())
    }

    /// Pop the pending event.
    fn pop_pending_event(&self, handle_event: &impl Fn(&Event) -> bool) -> bool {
        let mut lock = self.event_queue.lock();
        let Ok((token, _)) = lock.pop_used() else {
            return false;
        };

        if token >= QUEUE_SIZE {
            return false;
        }
        let event = self.event_buf.iter().nth(token as usize).unwrap();
        let res = handle_event(&event);
        // requeue
        // FIEME: replace slice with a more secure data structure to use dma mapping.
        let new_token = lock
            .add_dma_buf(&[], &[&(event.daddr(), event.size())])
            .unwrap();
        // This only works because nothing happen between `pop_used` and `add` that affects
        // the list of free descriptors in the queue, so `add` reuses the descriptor which
        // was just freed by `pop_used`.
        assert_eq!(new_token, token);
        res
    }

    /// Query a specific piece of information by `select` and `subsel`, and write
    /// result to `out`, return the result size.
    pub fn query_config_select(&self, select: InputConfigSelect, subsel: u8, out: &mut [u8]) -> u8 {
        field_ptr!(&self.config, VirtioInputConfig, select)
            .write(&(select as u8))
            .unwrap();
        field_ptr!(&self.config, VirtioInputConfig, subsel)
            .write(&subsel)
            .unwrap();
        let size = field_ptr!(&self.config, VirtioInputConfig, size)
            .read()
            .unwrap();
        let data: [u8; 128] = field_ptr!(&self.config, VirtioInputConfig, data)
            .read()
            .unwrap();
        out[..size as usize].copy_from_slice(&data[..size as usize]);
        size
    }

    /// Negotiate features for the device specified bits 0~23
    pub(crate) fn negotiate_features(features: u64) -> u64 {
        assert_eq!(features, 0);
        0
    }
}

#[derive(Debug)]
struct EventBuf(DmaStream);

impl EventBuf {
    fn new() -> Self {
        let vm_segment = {
            let mut options = VmAllocOptions::new(1);
            options.is_contiguous(true);
            options.alloc_contiguous().unwrap()
        };

        let default_event = VirtioInputEvent::default();
        for idx in 0..QUEUE_SIZE as usize {
            let offset = idx * core::mem::size_of::<VirtioInputEvent>();
            vm_segment.write_val(offset, &default_event).unwrap();
        }

        let stream = DmaStream::map(vm_segment, DmaDirection::FromDevice, false).unwrap();
        Self(stream)
    }

    fn iter(&self) -> EventBufIter {
        EventBufIter {
            event_buf: self,
            cur_idx: 0,
        }
    }
}

struct Event<'a> {
    event_buf: &'a EventBuf,
    offset: usize,
    size: usize,
}

impl<'a> Event<'a> {
    fn daddr(&self) -> Daddr {
        self.event_buf.0.daddr() + self.offset
    }

    const fn size(&self) -> usize {
        self.size
    }

    fn reader(&self) -> VmReader<'a> {
        self.event_buf
            .0
            .sync(self.offset..self.offset + self.size)
            .unwrap();
        self.event_buf
            .0
            .reader()
            .unwrap()
            .skip(self.offset)
            .limit(self.size)
    }
}

struct EventBufIter<'a> {
    event_buf: &'a EventBuf,
    cur_idx: usize,
}

impl<'a> Iterator for EventBufIter<'a> {
    type Item = Event<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cur_idx >= QUEUE_SIZE as usize {
            return None;
        }

        let event_size = core::mem::size_of::<VirtioInputEvent>();
        let offset = self.cur_idx * event_size;
        let event = Event {
            event_buf: self.event_buf,
            offset,
            size: event_size,
        };

        self.cur_idx += 1;

        Some(event)
    }
}

impl aster_input::InputDevice for InputDevice {
    fn handle_irq(&self) {
        // If there may be more events to handle
        let handle_event = |event: &Event| -> bool {
            let event: VirtioInputEvent = {
                let mut reader = event.reader();
                reader.read_val()
            };

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

            let event = InputEvent::KeyBoard(Key::try_from(event.code).unwrap(), status);
            info!("Input Event:{:?}", event);

            let callbacks = self.callbacks.lock();
            for callback in callbacks.iter() {
                callback(event);
            }

            true
        };

        // one interrupt may contains serval input, so it should loop
        while self.pop_pending_event(&handle_event) {}
    }

    fn register_callbacks(&self, function: &'static (dyn Fn(InputEvent) + Send + Sync)) {
        self.callbacks.lock().push(Arc::new(function))
    }
}

impl Debug for InputDevice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InputDevice")
            .field("config", &self.config)
            .field("event_queue", &self.event_queue)
            .field("status_queue", &self.status_queue)
            .field("event_buf", &self.event_buf)
            .field("transport", &self.transport)
            .finish()
    }
}
