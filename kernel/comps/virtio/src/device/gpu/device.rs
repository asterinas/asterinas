use alloc::{
    boxed::Box,
    sync::Arc,
};
use alloc::vec;
use log::{debug, info};
use ostd::early_println;
use ostd::task::scheduler::info;
use ostd::{
    sync::SpinLock,
    mm::{DmaDirection, DmaStream, DmaStreamSlice, FrameAllocOptions, HasPaddr, VmIo},
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

use crate::{
    device::{
        gpu::{
            control::{
                VirtioGPUFormats, VirtioGPUResourceCreate2D, VirtioGPUGetEdid, VirtioGPURespEdid,
                VirtioGPURespSetScanout, VirtioGPUSetScanout, VirtioGPURect, VirtioGPURespResourceCreate2D,
                VirtioGPUResourceAttachBacking, VirtioGPURespAttachBacking, VirtioGPUMemEntry, VirtioGPURespDisplayInfo
            },
            header::{VirtioGPUCtrlType},
        },
    }
};

use::core::hint::spin_loop;

pub struct GPUDevice {
    config_manager: ConfigManager<VirtioGPUConfig>,
    transport: SpinLock<Box<dyn VirtioTransport>>,
    control_queue: SpinLock<VirtQueue>,
    cursor_queue: SpinLock<VirtQueue>,
    control_request: DmaStream,
    control_response: DmaStream,
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
        let control_request = {
            let vm_segment = FrameAllocOptions::new().alloc_segment(1).unwrap();
            DmaStream::map(vm_segment.into(), DmaDirection::Bidirectional, false).unwrap()
        };
        let control_response = {
            let vm_segment = FrameAllocOptions::new().alloc_segment(1).unwrap();
            DmaStream::map(vm_segment.into(), DmaDirection::Bidirectional, false).unwrap()
        };

        // init device
        let device = Arc::new(Self {
            config_manager,
            transport: SpinLock::new(transport),
            control_queue,
            cursor_queue,
            control_request,
            control_response
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
        test_basic_config(Arc::clone(&device));
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

    fn get_display_info(&self) -> Result<VirtioGPURespDisplayInfo, VirtioDeviceError> {
        let req_slice = {
            let req_slice = DmaStreamSlice::new(&self.control_request, 0, size_of::<VirtioGPUCtrlHdr>());
            let req = VirtioGPUCtrlHdr {
                ctrl_type: VirtioGPUCtrlType::VIRTIO_GPU_CMD_GET_DISPLAY_INFO as u32,
                ..VirtioGPUCtrlHdr::default()
            };
            req_slice.write_val(0, &req).unwrap();
            req_slice.sync().unwrap();
            req_slice
        };

        let resp_slice = {
            let resp_slice = DmaStreamSlice::new(&self.control_response, 0, size_of::<VirtioGPURespDisplayInfo>());
            resp_slice.write_val(0, &VirtioGPURespDisplayInfo::default()).unwrap();
            resp_slice.sync().unwrap();
            resp_slice
        };

        let mut queue = self.control_queue.disable_irq().lock();
        let _token = queue
            .add_dma_buf(&[&req_slice], &[&resp_slice])
            .expect("add queue failed");
        if queue.should_notify() {
            queue.notify();
        }
        while !queue.can_pop() {
            spin_loop();
        }
        queue.pop_used().expect("pop used failed");
        resp_slice.sync().unwrap();
        let resp: VirtioGPURespDisplayInfo = resp_slice.read_val(0).unwrap();
        Ok(resp)
    }

    fn get_edid(&self) -> Result<(), VirtioDeviceError> {
        let req_slice = {
            let req_slice =
                DmaStreamSlice::new(&self.control_request, 0, size_of::<VirtioGPUGetEdid>());
            let req = VirtioGPUGetEdid::default();
            req_slice.write_val(0, &req).unwrap();
            req_slice.sync().unwrap();
            req_slice
        };

        let resp_slice = {
            let resp_slice =
                DmaStreamSlice::new(&self.control_response, 0, size_of::<VirtioGPURespEdid>());
            resp_slice.write_val(0, &VirtioGPURespEdid::default()).unwrap();
            resp_slice.sync().unwrap();
            resp_slice
        };
        
        let mut queue = self.control_queue.disable_irq().lock();
        let _token = queue
            .add_dma_buf(&[&req_slice], &[&resp_slice])
            .expect("add queue failed");
        if queue.should_notify() {
            queue.notify();
        }
        while !queue.can_pop() {
            spin_loop();
        }
        queue.pop_used_with_token(_token).expect("pop used failed");
        resp_slice.sync().unwrap();
        let resp: VirtioGPURespEdid  = resp_slice.read_val(0).unwrap();
        if resp.get_type() == VirtioGPUCtrlType::VIRTIO_GPU_RESP_OK_EDID as u32 {
            Ok(())
        } else {
            Err(VirtioDeviceError::QueueUnknownError)
        }
    }

    fn print_resolution(&self) {
        let display_info = self.get_display_info().unwrap();
        let rect = display_info.get_rect(0).unwrap();
        early_println!("width: {}, height: {}", rect.width, rect.height);
    }

    fn resource_create_2d(
        &self,
        resource_id: u32,
        width: u32,
        height: u32,
    ) -> Result<(), VirtioDeviceError> {
        // Resemble to ../block/device.rs
        let req_slice = {
            let req_slice = DmaStreamSlice::new(
                &self.control_request, 0, size_of::<VirtioGPUResourceCreate2D>());
            let req = VirtioGPUResourceCreate2D::new(
                resource_id,
                VirtioGPUFormats::VIRTIO_GPU_FORMAT_B8G8R8A8_UNORM,
                width,
                height,
            );
            req_slice.write_val(0, &req).unwrap();
            req_slice.sync().unwrap();
            req_slice
        };
        let resp_slice = {
            let resp_slice = DmaStreamSlice::new(
                &self.control_response, 0, size_of::<VirtioGPURespResourceCreate2D>());
            resp_slice.write_val(0, &VirtioGPURespResourceCreate2D::default()).unwrap();
            resp_slice.sync().unwrap();
            resp_slice
        };

        let mut queue = self.control_queue.disable_irq().lock();
        let _token = queue
            .add_dma_buf(&[&req_slice], &[&resp_slice])
            .expect("add queue failed");

        if queue.should_notify() {
            queue.notify();
        }
        while !queue.can_pop() {
            spin_loop();
        }
        queue.pop_used().expect("pop used failed");

        resp_slice.sync().unwrap();
        let resp: VirtioGPURespResourceCreate2D = resp_slice.read_val(0).unwrap();
        if resp.get_type() == VirtioGPUCtrlType::VIRTIO_GPU_RESP_OK_NODATA as u32 {
            Ok(())
        } else {
            Err(VirtioDeviceError::QueueUnknownError)
        }
    }

    fn resource_attach_backing(
        &self,
        resource_id: i32,
        paddr: usize,
        size: u32,
    ) -> Result<(), VirtioDeviceError> {
        let req_slice = {
            let req_slice = DmaStreamSlice::new(
                &self.control_request, 0, size_of::<VirtioGPUResourceAttachBacking>());
            let req = VirtioGPUResourceAttachBacking::new(resource_id as u32, 1);
            req_slice.write_val(0, &req).unwrap();
            req_slice.sync().unwrap();
            req_slice
        };

        let mem_slice = {
            let mem_slice = DmaStreamSlice::new(
                &self.control_request,
                size_of::<VirtioGPUResourceAttachBacking>(),
                size_of::<VirtioGPUMemEntry>(),
            );
            let mem = VirtioGPUMemEntry::new(paddr, size);
            mem_slice.write_val(0, &mem).unwrap();
            mem_slice.sync().unwrap();
            mem_slice
        };

        let resp_slice = {
            let resp_slice = DmaStreamSlice::new(
                &self.control_response,
                0,
                size_of::<VirtioGPURespAttachBacking>(),
            );
            resp_slice
                .write_val(0, &VirtioGPURespAttachBacking::default())
                .unwrap();
            resp_slice.sync().unwrap();
            resp_slice
        };

        let mut queue = self.control_queue.disable_irq().lock();
        let inputs = vec![&req_slice, &mem_slice];
        let _token = queue
            .add_dma_buf(inputs.as_slice(), &[&resp_slice])
            .expect("add queue failed");
        if queue.should_notify() {
            queue.notify();
        }
        while !queue.can_pop() {
            spin_loop();
        }
        queue.pop_used().expect("pop used failed");

        resp_slice.sync().unwrap();
        let resp: VirtioGPURespAttachBacking  = resp_slice.read_val(0).unwrap();
        if resp.get_type() == VirtioGPUCtrlType::VIRTIO_GPU_RESP_OK_NODATA as u32 {
            Ok(())
        } else {
            Err(VirtioDeviceError::QueueUnknownError)
        }
    }
    fn set_scanout(
        &self,
        rect: VirtioGPURect,
        scanout_id: i32,
        resource_id: i32,
    ) -> Result<(), VirtioDeviceError> {
        let req_slice = {
            let req_slice = DmaStreamSlice::new(
                &self.control_request, 0, size_of::<VirtioGPUSetScanout>());
            let req = VirtioGPUSetScanout::new(scanout_id as u32, resource_id as u32, rect);
            req_slice.write_val(0, &req).unwrap();
            req_slice.sync().unwrap();
            req_slice
        };

        let resp_slice = {
            let resp_slice = DmaStreamSlice::new(
                &self.control_response, 0, size_of::<VirtioGPURespSetScanout>());
            resp_slice
                .write_val(0, &VirtioGPURespSetScanout::default())
                .unwrap();
            resp_slice.sync().unwrap();
            resp_slice
        };

        let mut queue = self.control_queue.disable_irq().lock();
        let _token = queue
            .add_dma_buf(&[&req_slice], &[&resp_slice])
            .expect("add queue failed");

        if queue.should_notify() {
            queue.notify();
        }
        while !queue.can_pop() {
            spin_loop();
        }
        queue.pop_used().expect("pop used failed");

        resp_slice.sync().unwrap();
        let resp: VirtioGPURespSetScanout = resp_slice.read_val(0).unwrap();
        if resp.get_type() == VirtioGPUCtrlType::VIRTIO_GPU_RESP_OK_NODATA as u32 {
            Ok(())
        } else {
            Err(VirtioDeviceError::QueueUnknownError)
        }
    }
}

fn test_basic_config(d: Arc<GPUDevice>) {
    d.print_resolution();
    d.get_edid().unwrap();
}