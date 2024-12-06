use alloc::{boxed::Box, sync::Arc};
use spin::Spin;
use core::hint::spin_loop;

use ostd::{
    early_println,
    mm::{DmaDirection, DmaStream, DmaStreamSlice, FrameAllocOptions},
    sync::SpinLock,
};

use crate::{
    device::VirtioDeviceError,
    queue::VirtQueue,
    transport::{self, VirtioTransport},
};

pub struct GPUDevice {
    control_queue: SpinLock<VirtQueue>,
    cursor_queue: SpinLock<VirtQueue>,
    control_buffer: DmaStream,
    cursor_buffer: DmaStream,
    transport: SpinLock<Box<dyn VirtioTransport>>,
}

impl GPUDevice {
    pub fn negotiate_features(features: u64) -> u64 {
        features
    }

    pub fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        early_println!("Init GPU device");
        return Ok(());

        // Initalize the control virtqueue
        const CONTROL_QUEUE_INDEX: u16 = 0;
        let control_queue =
            SpinLock::new(VirtQueue::new(CONTROL_QUEUE_INDEX, 1, transport.as_mut()).unwrap());

        // Initalize the cursor virtqueue
        const CURSOR_QUEUE_INDEX: u16 = 1;
        let cursor_queue =
            SpinLock::new(VirtQueue::new(CURSOR_QUEUE_INDEX, 1, transport.as_mut()).unwrap());

        // Initalize the control buffer
        let control_buffer = {
            let vm_segment = FrameAllocOptions::new(1).alloc_contiguous().unwrap();
            DmaStream::map(vm_segment, DmaDirection::FromDevice, false).unwrap()
        };

        // Initalize the cursor buffer
        let cursor_buffer = {
            let vm_segment = FrameAllocOptions::new(1).alloc_contiguous().unwrap();
            DmaStream::map(vm_segment, DmaDirection::FromDevice, false).unwrap()
        };

        // Create device
        let device = Arc::new(Self {
            control_queue,
            cursor_queue,
            control_buffer,
            cursor_buffer,
            transport: SpinLock::new(transport),
        });
        // Finish init
        device.transport.lock().finish_init();

        // Test device
        test_device(device);
        Ok(())
    }
}

fn test_device(device: Arc<GPUDevice>) {
    unimplemented!()
}