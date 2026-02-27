// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, format, string::String, sync::Arc};
use core::sync::atomic::{AtomicUsize, Ordering};

use aster_util::mem_obj_slice::Slice;
use log::debug;
use ostd::{
    Error,
    arch::trap::TrapFrame,
    mm::{
        FallibleVmRead, PAGE_SIZE, VmWriter,
        dma::{DmaStream, FromDevice},
        io_util::HasVmReaderWriter,
    },
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

const ENTROPY_QUEUE_SIZE: u16 = 64;
const ENTROPY_BUFFER_SIZE: usize = PAGE_SIZE;
const ENTROPY_BUFFER_COUNT: usize = ENTROPY_QUEUE_SIZE as usize;

pub static RNG_CURRENT: Once<Mutex<String>> = Once::new();

pub struct EntropyDevice {
    transport: SpinLock<Box<dyn VirtioTransport>>,
    request_queue: SpinLock<VirtQueue>,
    receive_buffer: DmaStream<FromDevice>,
}

impl EntropyDevice {
    pub fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let request_queue =
            SpinLock::new(VirtQueue::new(0, ENTROPY_QUEUE_SIZE, transport.as_mut()).unwrap());
        let receive_buffer = DmaStream::alloc_uninit(ENTROPY_BUFFER_COUNT, false).unwrap();

        let device = Arc::new(EntropyDevice {
            transport: SpinLock::new(transport),
            request_queue,
            receive_buffer,
        });

        device.active_receive_buffer();

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

    pub fn try_read(&self, writer: &mut VmWriter) -> Result<usize, (Error, usize)> {
        let mut queue = self.request_queue.disable_irq().lock();
        let Ok((token, used_len)) = queue.pop_used() else {
            return Ok(0);
        };

        let buffer_index = token as usize;
        let used_len = (used_len as usize).min(ENTROPY_BUFFER_SIZE);
        let range = Self::buffer_range(buffer_index);

        self.receive_buffer
            .sync_from_device(range.start..range.start + used_len)
            .unwrap();

        let to_read = used_len.min(writer.avail());
        let mut reader = self.receive_buffer.reader().unwrap();
        reader.skip(range.start);
        reader.limit(to_read);

        let read_len = reader.read_fallible(writer)?;

        queue
            .add_dma_buf(&[], &[&Slice::new(&self.receive_buffer, range)])
            .unwrap();

        if queue.should_notify() {
            queue.notify();
        }

        Ok(read_len)
    }

    fn active_receive_buffer(&self) {
        let mut queue = self.request_queue.disable_irq().lock();

        for buffer_index in 0..ENTROPY_BUFFER_COUNT {
            let range = Self::buffer_range(buffer_index);

            queue
                .add_dma_buf(&[], &[&Slice::new(&self.receive_buffer, range)])
                .unwrap();

            if queue.should_notify() {
                queue.notify();
            }
        }
    }

    fn buffer_range(buffer_index: usize) -> core::ops::Range<usize> {
        let start = buffer_index * ENTROPY_BUFFER_SIZE;
        start..start + ENTROPY_BUFFER_SIZE
    }
}

fn config_space_change(_: &TrapFrame) {
    debug!("Virtio-Entropy device configuration space change");
}
