// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, format, string::String, sync::Arc};
use core::sync::atomic::{AtomicUsize, Ordering};

use aster_util::mem_obj_slice::Slice;
use log::debug;
use ostd::{
    arch::trap::TrapFrame,
    mm::dma::{DmaStream, FromDevice},
    sync::{Mutex, SpinLock},
};
use spin::Once;

use crate::{
    device::{
        VirtioDeviceError,
        entropy::{handle_recv_irq, register_device},
    },
    queue::VirtQueue,
    transport::VirtioTransport,
};

pub static ENTROPY_DEVICE_PREFIX: &str = "virtio_rng.";
static ENTROPY_DEVICE_ID: AtomicUsize = AtomicUsize::new(0);

pub static RNG_CURRENT: Once<Mutex<String>> = Once::new();

pub struct EntropyDevice {
    transport: SpinLock<Box<dyn VirtioTransport>>,
    pub request_queue: SpinLock<VirtQueue>,
    pub receive_buffer: DmaStream<FromDevice>,
}

impl EntropyDevice {
    pub fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let request_queue = SpinLock::new(VirtQueue::new(0, 1, transport.as_mut()).unwrap());
        let receive_buffer = DmaStream::alloc_uninit(1, false).unwrap();

        let device = Arc::new(EntropyDevice {
            transport: SpinLock::new(transport),
            request_queue,
            receive_buffer,
        });

        // Register irq callbacks
        let mut transport = device.transport.disable_irq().lock();
        transport
            .register_queue_callback(0, Box::new(handle_recv_irq), false)
            .unwrap();
        transport
            .register_cfg_callback(Box::new(config_space_change))
            .unwrap();
        transport.finish_init();
        drop(transport);

        let device_id = ENTROPY_DEVICE_ID.fetch_add(1, Ordering::SeqCst);
        let name = format!("{ENTROPY_DEVICE_PREFIX}{device_id}");

        register_device(name.clone(), device);

        RNG_CURRENT.call_once(|| Mutex::new(name));

        Ok(())
    }

    pub fn can_pop(&self) -> bool {
        let request_queue = self.request_queue.lock();
        request_queue.can_pop()
    }

    pub fn activate_receive_buffer(&self, receive_queue: &mut VirtQueue, to_read: usize) {
        receive_queue
            .add_dma_buf(&[], &[&Slice::new(&self.receive_buffer, 0..to_read)])
            .unwrap();

        if receive_queue.should_notify() {
            receive_queue.notify();
        }
    }
}

fn config_space_change(_: &TrapFrame) {
    debug!("Virtio-Entropy device configuration space change");
}
