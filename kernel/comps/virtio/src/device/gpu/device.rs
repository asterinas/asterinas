use alloc::{boxed::Box, sync::Arc};
use core::hint::spin_loop;

use log::info;
use ostd::{
    early_println,
    mm::{DmaDirection, DmaStream, DmaStreamSlice, FrameAllocOptions, VmIo},
    sync::SpinLock,
    trap::TrapFrame,
};

use super::{
    config::{GPUFeatures, VirtioGPUConfig},
    control::VirtioGpuRespDisplayInfo,
    header::VirtioGpuCtrlHdr,
};
use crate::{
    device::{
        gpu::{
            control::RESPONSE_SIZE,
            header::{VirtioGpuCtrlType, REQUEST_SIZE},
        },
        VirtioDeviceError,
    },
    queue::VirtQueue,
    transport::{ConfigManager, VirtioTransport},
};

/// Both virtqueues have the same format.
/// Each request and each response have a fixed header, followed by command specific data fields. See header.rs for the header format.
pub struct GPUDevice {
    config_manager: ConfigManager<VirtioGPUConfig>,

    /// queue for sending control commands
    control_queue: SpinLock<VirtQueue>,
    /// queue for sending cursor updates.
    /// According to virtio spec v1.3, 5.7.2 Virtqueues:
    /// The separate cursor queue is the "fast track" for cursor commands (VIRTIO_GPU_CMD_UPDATE_CURSOR and VIRTIO_GPU_CMD_MOVE_CURSOR),
    /// so they go through without being delayed by time-consuming commands in the control queue.
    cursor_queue: SpinLock<VirtQueue>,

    // request and response DMA buffer for control queue
    control_request: DmaStream,
    control_response: DmaStream,

    // TODO: (Taojie): add buffers for cursor queue

    // Since the virtio gpu header remains consistent for both requests and responses,
    // we store it to avoid recreating the header repeatedly.
    header: VirtioGpuCtrlHdr,
    transport: SpinLock<Box<dyn VirtioTransport>>,
}

impl GPUDevice {
    const QUEUE_SIZE: u16 = 64;

    pub fn negotiate_features(features: u64) -> u64 {
        let mut features = GPUFeatures::from_bits_truncate(features);
        early_println!("virtio_gpu_features = {:?}", features);
        features.bits()
    }

    pub fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let config_manager = VirtioGPUConfig::new_manager(transport.as_ref());
        early_println!("virtio_gpu_config = {:?}", config_manager.read_config());

        // Initalize virtqueues
        const CONTROL_QUEUE_INDEX: u16 = 0;
        const CURSOR_QUEUE_INDEX: u16 = 1;
        // TODO(Taojie): the size of queues?
        let control_queue = SpinLock::new(
            VirtQueue::new(CONTROL_QUEUE_INDEX, Self::QUEUE_SIZE, transport.as_mut())
                .expect("create control queue failed"),
        );
        let cursor_queue = SpinLock::new(
            VirtQueue::new(CURSOR_QUEUE_INDEX, Self::QUEUE_SIZE, transport.as_mut())
                .expect("create cursor queue failed"),
        );

        // Initalize DMA buffers
        let control_request = {
            let vm_segment = FrameAllocOptions::new(1).alloc_contiguous().unwrap();
            DmaStream::map(vm_segment, DmaDirection::Bidirectional, false).unwrap()
        };
        let control_response = {
            let vm_segment = FrameAllocOptions::new(1).alloc_contiguous().unwrap();
            DmaStream::map(vm_segment, DmaDirection::Bidirectional, false).unwrap()
        };

        // Create device
        let device = Arc::new(Self {
            config_manager,
            control_queue,
            cursor_queue,
            control_request,
            control_response,
            header: VirtioGpuCtrlHdr::default(),
            transport: SpinLock::new(transport),
        });

        // Interrupt handler
        let clone_device = device.clone();
        let handle_irq = move |_: &TrapFrame| {
            clone_device.handle_irq();
        };

        let clone_device = device.clone();
        let handle_config_change = move |_: &TrapFrame| {
            clone_device.handle_config_change();
        };

        // Register irq callbacks
        let mut transport = device.transport.lock();
        transport
            .register_queue_callback(CONTROL_QUEUE_INDEX, Box::new(handle_irq), false)
            .unwrap();
        transport
            .register_cfg_callback(Box::new(handle_config_change))
            .unwrap();

        transport.finish_init();

        // Done: query the display information from the device using the VIRTIO_GPU_CMD_GET_DISPLAY_INFO command,
        //      and use that information for the initial scanout setup.

        // TODO: fetch the EDID information using the VIRTIO_GPU_CMD_GET_EDID command,
        //      If no information is available or all displays are disabled the driver MAY choose to use a fallback, such as 1024x768 at display 0.

        // TODO: query all shared memory regions supported by the device.
        //      If the device supports shared memory, the shmid of a region MUST be one of:
        //      - VIRTIO_GPU_SHM_ID_UNDEFINED  = 0
        //      - VIRTIO_GPU_SHM_ID_HOST_VISIBLE = 1

        // Test device
        // test_device(device);
        device.request_display_info();

        Ok(())
    }

    fn handle_config_change(&self) {
        info!("virtio_gpu: config space change");
    }

    fn handle_irq(&self) {
        info!("virtio_gpu handle irq");
        // TODO: follow the implementation of virtio_block
    }

    fn request_display_info(&self) -> Result<(), VirtioDeviceError> {
        // Prepare request DMA buffer
        let req_slice = {
            let req_slice = DmaStreamSlice::new(&self.control_request, 0, REQUEST_SIZE);
            let req = VirtioGpuCtrlHdr {
                type_: VirtioGpuCtrlType::VIRTIO_GPU_CMD_GET_DISPLAY_INFO as u32,
                ..VirtioGpuCtrlHdr::default()
            };
            req_slice.write_val(0, &req).unwrap();
            req_slice.sync().unwrap();
            req_slice
        };

        // Prepare response DMA buffer
        let resp_slice = {
            let resp_slice = DmaStreamSlice::new(&self.control_response, 0, RESPONSE_SIZE);
            resp_slice
                .write_val(0, &VirtioGpuRespDisplayInfo::default())
                .unwrap();
            resp_slice.sync().unwrap();
            resp_slice
        };

        // Add buffer to queue
        let mut control_queue = self.control_queue.disable_irq().lock();
        control_queue
            .add_dma_buf(&[&req_slice], &[&resp_slice])
            .expect("Add buffers to queue failed");

        // Notify
        if control_queue.should_notify() {
            control_queue.notify();
        }

        // Wait for response
        while !control_queue.can_pop() {
            early_println!("waiting for response...");
            spin_loop();
        }
        control_queue.pop_used().expect("Pop used failed");

        resp_slice.sync().unwrap();
        let resp: VirtioGpuRespDisplayInfo = resp_slice.read_val(0).unwrap();
        early_println!("display info from virt_gpu device: {:?}", resp);
        Ok(())
    }
}

fn test_device(device: Arc<GPUDevice>) {
    unimplemented!()
}
