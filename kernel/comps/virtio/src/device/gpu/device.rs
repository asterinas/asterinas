use alloc::{
    boxed::Box,
    sync::Arc,
};
use log::{debug, info};
use ostd::early_println;
use ostd::task::scheduler::info;
use ostd::{
    sync::SpinLock,
    mm::{DmaDirection, DmaStream, DmaStreamSlice, FrameAllocOptions},
    trap::TrapFrame,
};
use crate::{
    device::VirtioDeviceError, 
    queue::VirtQueue, 
    transport::{ConfigManager, VirtioTransport}
};

use super::{
    config::{GPUFeatures, VirtioGPUConfig},
    header::VirtioGPUCtrlHdr,
};

pub struct GPUDevice {
    config_manager: ConfigManager<VirtioGPUConfig>,
    transport: SpinLock<Box<dyn VirtioTransport>>,
    control_queue: SpinLock<VirtQueue>,
    cursor_queue: SpinLock<VirtQueue>,
    controlq_receiver: DmaStream,
    controlq_sender: DmaStream,
    // cursor_receiver: DmaStream,          // TODO: ?
    // cursor_sender: DmaStream,
    // callback                             // FIXME: necessary?
}

impl GPUDevice {
    const QUEUE_SIZE: u16 = 64;

    pub fn negotiate_features(features: u64) -> u64 {
        let mut features = GPUFeatures::from_bits_truncate(features);
        debug!("GPUFeature negotiate: {:?}", features);
        // tmep: not support 3D mode
        features.remove(GPUFeatures::VIRTIO_GPU_F_VIRGL);
        features.remove(GPUFeatures::VIRTIO_GPU_F_CONTEXT_INIT);
        features.bits()
    }

    pub fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let config_manager = VirtioGPUConfig::new_manager(transport.as_ref());
        early_println!("[INFO] GPU Config = {:?}", config_manager.read_config());

        // init queue
        const CONTROL_QUEUE_INDEX: u16 = 0;
        const CURSOR_QUEUE_INDEX: u16 = 1;
        let control_queue =
            SpinLock::new(VirtQueue::new(CONTROL_QUEUE_INDEX, Self::QUEUE_SIZE, transport.as_mut()).unwrap());
        let cursor_queue =
            SpinLock::new(VirtQueue::new(CURSOR_QUEUE_INDEX, Self::QUEUE_SIZE, transport.as_mut()).unwrap());

        // init buffer
        let controlq_receiver = {
            let segment = FrameAllocOptions::new().alloc_segment(1).unwrap();
            DmaStream::map(segment.into(), DmaDirection::ToDevice, false).unwrap()
        };
        let controlq_sender = {
            let segment = FrameAllocOptions::new().alloc_segment(1).unwrap();
            DmaStream::map(segment.into(), DmaDirection::FromDevice, false).unwrap()
        };

        // init device
        let device = Arc::new(Self {
            config_manager,
            transport: SpinLock::new(transport),
            control_queue,
            cursor_queue,
            controlq_receiver,
            controlq_sender
            // TODO: ...
        });

        // Handle interrupt (ref. block device)
        let cloned_device = device.clone();
        let handle_irq = move |_: &TrapFrame| {
            cloned_device.handle_irq();
        };

        let cloned_device = device.clone();
        let handle_config_change = move |_: &TrapFrame| {
            cloned_device.handle_config_change();
        };

        // Register callback
        let mut transport = device.transport.lock();
        transport
            .register_cfg_callback(Box::new(handle_config_change))
            .unwrap();
        transport
            .register_queue_callback(0, Box::new(handle_irq), false)
            .unwrap();
        transport.finish_init();

        Ok(())
    }

    fn handle_irq(&self) {
        info!("Virtio-GPU handle irq");
        // TODO
    }

    fn handle_config_change(&self) {
        info!("Virtio-GPU handle config change");
        // TODO
    }
}

