// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc};
use core::{
    hint::spin_loop,
    sync::atomic::{AtomicUsize, Ordering},
};

use aster_util::mem_obj_slice::Slice;
use ostd::{
    mm::{
        VmWriter,
        dma::{DmaStream, FromDevice},
        io_util::HasVmReaderWriter,
    },
    sync::SpinLock,
};

use crate::{
    device::{VirtioDeviceError, entropy::register_device},
    queue::VirtQueue,
    transport::VirtioTransport,
};

static ENTROPY_DEVICE_ID: AtomicUsize = AtomicUsize::new(0);

pub struct EntropyDevice {
    request_queue: SpinLock<VirtQueue>,
    receive_buffer: DmaStream<FromDevice>,
}

impl EntropyDevice {
    pub fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let request_queue = SpinLock::new(VirtQueue::new(0, 1, transport.as_mut()).unwrap());

        let receive_buffer = DmaStream::alloc_uninit(1, false).unwrap();

        let device = EntropyDevice {
            request_queue,
            receive_buffer,
        };

        transport.finish_init();

        let device_id = ENTROPY_DEVICE_ID.fetch_add(1, Ordering::SeqCst);

        register_device(device_id, Arc::new(device));

        Ok(())
    }

    /// The caller must ensure that the `buf` size is not larger than `PAGE_SIZE`.
    pub fn getrandom(&self, buf: &mut [u8]) {
        let mut request_queue = self.request_queue.disable_irq().lock();

        let mut read_bytes = 0;
        while read_bytes < buf.len() {
            let to_read = buf.len() - read_bytes;
            let slice = Slice::new(&self.receive_buffer, 0..to_read);

            request_queue
                .add_dma_buf(&[], &[&slice])
                .expect("Failed to add DMA buffer to entropy request queue");

            if request_queue.should_notify() {
                request_queue.notify();
            }

            while !request_queue.can_pop() {
                spin_loop();
            }

            let used_elem = request_queue.pop_used().unwrap();
            let len = used_elem.1 as usize;

            self.receive_buffer.sync_from_device(0..len).unwrap();

            let mut reader = self.receive_buffer.reader().unwrap();
            reader.read(&mut VmWriter::from(&mut buf[read_bytes..read_bytes + len]));

            read_bytes += len;
        }
    }
}
