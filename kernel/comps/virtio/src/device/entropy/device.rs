// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, format, sync::Arc};
use core::sync::atomic::{AtomicUsize, Ordering};

use aster_network::dma_pool::{DmaPool, DmaSegment};
use aster_util::slot_vec::SlotVec;
use log::debug;
use ostd::{
    arch::trap::TrapFrame,
    mm::{PAGE_SIZE, dma::FromDevice},
    sync::SpinLock,
};

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

const POOL_INIT_SIZE: usize = 0;
const POOL_HIGH_WATERMARK: usize = 64;
const ENTROPY_QUEUE_SIZE: u16 = 64;
const ENTROPY_BUFFER_SIZE: usize = PAGE_SIZE;

/// Entropy devices, which supply high-quality randomness for guest use.
pub struct EntropyDevice {
    transport: SpinLock<Box<dyn VirtioTransport>>,
    request_queue: SpinLock<EntropyRequestQueue>,
}

impl EntropyDevice {
    pub(crate) fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let receive_buffer_pool = DmaPool::new(
            ENTROPY_BUFFER_SIZE,
            POOL_INIT_SIZE,
            POOL_HIGH_WATERMARK,
            false,
        );
        let request_queue = SpinLock::new(EntropyRequestQueue::new(
            VirtQueue::new(0, ENTROPY_QUEUE_SIZE, transport.as_mut()).unwrap(),
            receive_buffer_pool,
        ));

        let mut device = EntropyDevice {
            transport: SpinLock::new(transport),
            request_queue,
        };

        // Register IRQ callbacks.
        let transport = device.transport.get_mut();
        transport
            .register_queue_callback(0, Box::new(handle_recv_irq), false)
            .unwrap();
        transport
            .register_cfg_callback(Box::new(config_space_change))
            .unwrap();
        transport.finish_init();

        let device_id = ENTROPY_DEVICE_ID.fetch_add(1, Ordering::Relaxed);
        let name = format!("{ENTROPY_DEVICE_PREFIX}{device_id}");

        register_device(name, Arc::new(device));

        Ok(())
    }

    /// Tries to read some random data from the device.
    pub fn try_read(&self) -> Option<(DmaSegment<FromDevice>, usize)> {
        let (receive_buffer, used_len) = {
            let mut request_queue = self.request_queue.lock();
            let (receive_buffer, used_len) = request_queue.pop_used()?;

            (receive_buffer, used_len)
        };

        receive_buffer.sync_from_device(0..used_len).unwrap();

        Some((receive_buffer, used_len))
    }
}

struct EntropyRequestQueue {
    queue: VirtQueue,
    receive_buffer_pool: Arc<DmaPool<FromDevice>>,
    receive_buffers: SlotVec<DmaSegment<FromDevice>>,
}

impl EntropyRequestQueue {
    fn new(queue: VirtQueue, receive_buffer_pool: Arc<DmaPool<FromDevice>>) -> Self {
        let mut this = Self {
            queue,
            receive_buffer_pool,
            receive_buffers: SlotVec::new(),
        };

        for _ in 0..ENTROPY_QUEUE_SIZE {
            let receive_buffer = this.receive_buffer_pool.alloc_segment().unwrap();
            let token = this.queue.add_dma_buf(&[], &[&receive_buffer]).unwrap();
            this.receive_buffers.put_at(token as usize, receive_buffer);
        }
        if this.queue.should_notify() {
            this.queue.notify();
        }

        this
    }

    fn pop_used(&mut self) -> Option<(DmaSegment<FromDevice>, usize)> {
        let Ok((token, used_len)) = self.queue.pop_used() else {
            return None;
        };

        let receive_buffer = self.receive_buffers.remove(token as usize).unwrap();

        let replenished_receive_buffer = self.receive_buffer_pool.alloc_segment().unwrap();
        self.add_receive_buffer_and_notify(replenished_receive_buffer);

        Some((receive_buffer, used_len as usize))
    }

    fn add_receive_buffer_and_notify(&mut self, receive_buffer: DmaSegment<FromDevice>) {
        let token = self.queue.add_dma_buf(&[], &[&receive_buffer]).unwrap();
        self.receive_buffers.put_at(token as usize, receive_buffer);

        if self.queue.should_notify() {
            self.queue.notify();
        }
    }
}

fn config_space_change(_: &TrapFrame) {
    debug!("Virtio-Entropy device configuration space change");
}
