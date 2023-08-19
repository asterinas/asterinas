use crate::{device::VirtioDeviceError, queue::VirtQueue, VirtioPciCommonCfg};
use alloc::{boxed::Box, vec::Vec};
use bitflags::bitflags;
use jinux_frame::sync::Mutex;
use jinux_frame::{io_mem::IoMem, offset_of};
use jinux_pci::{capability::vendor::virtio::CapabilityVirtioData, util::BAR};
use jinux_util::{field_ptr, safe_ptr::SafePtr};
use pod::Pod;

use super::{
    InputConfigSelect, InputEvent, VirtioInputConfig, QUEUE_EVENT, QUEUE_SIZE, QUEUE_STATUS,
};

bitflags! {
    pub struct InputProp : u8{
        const POINTER           = 1 << 0;
        const DIRECT            = 1 << 1;
        const BUTTONPAD         = 1 << 2;
        const SEMI_MT           = 1 << 3;
        const TOPBUTTONPAD      = 1 << 4;
        const POINTING_STICK    = 1 << 5;
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

/// Virtual human interface devices such as keyboards, mice and tablets.
///
/// An instance of the virtio device represents one such input device.
/// Device behavior mirrors that of the evdev layer in Linux,
/// making pass-through implementations on top of evdev easy.
#[derive(Debug)]
pub struct InputDevice {
    config: SafePtr<VirtioInputConfig, IoMem>,
    event_queue: Mutex<VirtQueue>,
    status_queue: VirtQueue,
    pub event_buf: Mutex<Box<[InputEvent; QUEUE_SIZE]>>,
}

impl InputDevice {
    /// Create a new VirtIO-Input driver.
    /// msix_vector_left should at least have one element or n elements where n is the virtqueue amount
    pub fn new(
        cap: &CapabilityVirtioData,
        bars: [Option<BAR>; 6],
        common_cfg: &SafePtr<VirtioPciCommonCfg, IoMem>,
        notify_base_address: usize,
        notify_off_multiplier: u32,
        mut msix_vector_left: Vec<u16>,
    ) -> Result<Self, VirtioDeviceError> {
        let mut event_buf = Box::new([InputEvent::default(); QUEUE_SIZE]);
        let vector_left = msix_vector_left.len();
        let mut next_msix_vector = msix_vector_left.pop().unwrap();
        let mut event_queue = VirtQueue::new(
            &common_cfg,
            QUEUE_EVENT,
            QUEUE_SIZE as u16,
            notify_base_address,
            notify_off_multiplier,
            next_msix_vector,
        )
        .expect("create event virtqueue failed");
        next_msix_vector = if vector_left == 1 {
            next_msix_vector
        } else {
            msix_vector_left.pop().unwrap()
        };
        let status_queue = VirtQueue::new(
            &common_cfg,
            QUEUE_STATUS,
            QUEUE_SIZE as u16,
            notify_base_address,
            notify_off_multiplier,
            next_msix_vector,
        )
        .expect("create status virtqueue failed");

        for (i, event) in event_buf.as_mut().iter_mut().enumerate() {
            let token = event_queue.add(&[], &[event.as_bytes_mut()]);
            match token {
                Ok(value) => {
                    assert_eq!(value, i as u16);
                }
                Err(_) => {
                    return Err(VirtioDeviceError::QueueUnknownError);
                }
            }
        }

        Ok(Self {
            config: VirtioInputConfig::new(cap, bars),
            event_queue: Mutex::new(event_queue),
            status_queue,
            event_buf: Mutex::new(event_buf),
        })
    }

    // /// Acknowledge interrupt and process events.
    // pub fn ack_interrupt(&mut self) -> bool {
    //     self.transport.ack_interrupt()
    // }

    /// Pop the pending event.
    pub fn pop_pending_event(&self) -> Option<InputEvent> {
        let mut lock = self.event_queue.lock();
        if let Ok((token, _)) = lock.pop_used() {
            if token >= QUEUE_SIZE as u16 {
                return None;
            }
            let event = &mut self.event_buf.lock()[token as usize];
            // requeue
            if let Ok(new_token) = lock.add(&[], &[event.as_bytes_mut()]) {
                // This only works because nothing happen between `pop_used` and `add` that affects
                // the list of free descriptors in the queue, so `add` reuses the descriptor which
                // was just freed by `pop_used`.
                assert_eq!(new_token, token);
                return Some(*event);
            }
        }
        None
    }

    /// Query a specific piece of information by `select` and `subsel`, and write
    /// result to `out`, return the result size.
    pub fn query_config_select(&self, select: InputConfigSelect, subsel: u8, out: &mut [u8]) -> u8 {
        field_ptr!(&self.config, VirtioInputConfig, select)
            .write(&(select as u8))
            .unwrap();
        field_ptr!(&self.config, VirtioInputConfig, subsel)
            .write(&(subsel as u8))
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
