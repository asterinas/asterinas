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
use crate::device::gpu::GPU_DEVICE;

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
                VirtioGPUResourceAttachBacking, VirtioGPURespAttachBacking, VirtioGPUMemEntry, VirtioGPURespDisplayInfo,
                VirtioGPUTransferToHost2D, VirtioGPURespTransferToHost2D,
                VirtioGPUResourceFlush, VirtioGPURespResourceFlush,
                VirtioGPUCursorPos, VirtioGPUUpdateCursor, VirtioGPURespUpdateCursor,
            },
            header::{VirtioGPUCtrlType, kBlockSize},
        },
    }
};

use::core::hint::spin_loop;

use tinybmp::Bmp;
use embedded_graphics::pixelcolor::Rgb888;
use alloc::vec::Vec;

pub struct GPUDevice {
    config_manager: ConfigManager<VirtioGPUConfig>,
    transport: SpinLock<Box<dyn VirtioTransport>>,
    control_queue: SpinLock<VirtQueue>,
    cursor_queue: SpinLock<VirtQueue>,
    control_request: DmaStream,
    control_response: DmaStream,
    cursor_request: DmaStream,
    cursor_response: DmaStream,
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
        let cursor_request = {
            let vm_segment = FrameAllocOptions::new().alloc_segment(1).unwrap();
            DmaStream::map(vm_segment.into(), DmaDirection::Bidirectional, false).unwrap()
        };
        let cursor_response = {
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
            control_response,
            cursor_request,
            cursor_response
        });

        // Handle interrupt (ref. block device)
        let cloned_device = device.clone();
        let handle_irq = move |_: &TrapFrame| {
            cloned_device.handle_irq();
        };
        let clone_device = device.clone();
        let handle_irq_cursor = move |_: &TrapFrame| {
            clone_device.handle_irq();
        };
        let cloned_device = device.clone();
        let handle_config_change = move |_: &TrapFrame| {
            cloned_device.handle_config_change();
        };

        // Register callback
        let mut transport = device.transport.lock();
        transport
            .register_queue_callback(0, Box::new(handle_irq), false)
            .unwrap();
        transport
            .register_queue_callback(1, Box::new(handle_irq_cursor), false)
            .unwrap();
        transport
            .register_cfg_callback(Box::new(handle_config_change))
            .unwrap();
        transport.finish_init();
        drop(transport);
        /* Create framebuffer */
        let addr1: u32 = 0x1111;
        let display_info = device.get_display_info().unwrap();
        let rect = display_info.get_rect(0).unwrap();
        early_println!("width: {}, height: {}", rect.width, rect.height);
        device.resource_create_2d(addr1, rect.width, rect.height).unwrap();
        let byte_cnt = rect.width * rect.height * 4 as u32;
        let frame_cnt = (byte_cnt + kBlockSize - 1) / kBlockSize as u32;
        let frames = {
            let segment = FrameAllocOptions::new().alloc_segment(frame_cnt as usize).unwrap();
            DmaStream::map(segment.into(), DmaDirection::ToDevice, false).unwrap()
        };
        device.resource_attach_backing(addr1, frames.paddr(), byte_cnt);
        device.set_scanout(rect, 0, addr1);
        for i in 0..rect.width {
            for j in 0..rect.height {
                let idx = (j * rect.width + i) * 4 as u32;
                frames.write_val(idx as usize, &(i + 2 * j)).unwrap();
                frames.write_val((idx + 1) as usize, &(2 * i + j)).unwrap();
                frames.write_val((idx + 2) as usize, &i).unwrap();
                frames.write_val((idx + 3) as usize, &j).unwrap();
            }
        }
        device.transfer_to_host_2d(rect, 0, addr1).unwrap();
        device.resource_flush(rect, addr1).unwrap();
        early_println!("flushed");
        GPU_DEVICE.call_once(|| SpinLock::new(device));
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
        queue.pop_used_with_token(_token).expect("pop used failed");
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
        queue.pop_used_with_token(_token).expect("pop used failed");

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
        resource_id: u32,
        paddr: usize,
        size: u32,
    ) -> Result<(), VirtioDeviceError> {
        let req_slice = {
            let req_slice = DmaStreamSlice::new(
                &self.control_request, 0, size_of::<VirtioGPUResourceAttachBacking>());
            let req = VirtioGPUResourceAttachBacking::new(resource_id, 1);
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
        queue.pop_used_with_token(_token).expect("pop used failed");
        resp_slice.sync().unwrap();
        let resp: VirtioGPURespAttachBacking = resp_slice.read_val(0).unwrap();
        if resp.get_type() == VirtioGPUCtrlType::VIRTIO_GPU_RESP_OK_NODATA as u32 {
            Ok(())
        } else {
            Err(VirtioDeviceError::QueueUnknownError)
        }
    }
    fn set_scanout(
        &self,
        rect: VirtioGPURect,
        scanout_id: u32,
        resource_id: u32,
    ) -> Result<(), VirtioDeviceError> {
        let req_slice = {
            let req_slice = DmaStreamSlice::new(
                &self.control_request, 0, size_of::<VirtioGPUSetScanout>());
            let req = VirtioGPUSetScanout::new(scanout_id, resource_id, rect);
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
        queue.pop_used_with_token(_token).expect("pop used failed");
        resp_slice.sync().unwrap();
        let resp: VirtioGPURespSetScanout = resp_slice.read_val(0).unwrap();
        if resp.get_type() == VirtioGPUCtrlType::VIRTIO_GPU_RESP_OK_NODATA as u32 {
            Ok(())
        } else {
            Err(VirtioDeviceError::QueueUnknownError)
        }
    }
    fn transfer_to_host_2d(
        &self,
        rect: VirtioGPURect,
        offset: u32,
        resource_id: u32,
    ) -> Result<(), VirtioDeviceError> {
        let req_slice = {
            let req_slice = DmaStreamSlice::new(
                &self.control_request, 0, size_of::<VirtioGPUTransferToHost2D>());
            let req = VirtioGPUTransferToHost2D::new(rect, offset as u64, resource_id);
            req_slice.write_val(0, &req).unwrap();
            req_slice.sync().unwrap();
            req_slice
        };
        let resp_slice = {
            let resp_slice = DmaStreamSlice::new(
                &self.control_response, 0, size_of::<VirtioGPURespTransferToHost2D>());
            resp_slice.write_val(0, &VirtioGPURespTransferToHost2D::default()).unwrap();
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
        let resp: VirtioGPURespSetScanout = resp_slice.read_val(0).unwrap();
        if resp.get_type() == VirtioGPUCtrlType::VIRTIO_GPU_RESP_OK_NODATA as u32 {
            Ok(())
        } else {
            Err(VirtioDeviceError::QueueUnknownError)
        }
    }
    fn resource_flush(&self, rect: VirtioGPURect, resource_id: u32) -> Result<(), VirtioDeviceError> {
        let req_slice = {
            let req_slice = DmaStreamSlice::new(
                &self.control_request, 0, size_of::<VirtioGPUResourceFlush>());
            let req = VirtioGPUResourceFlush::new(rect, resource_id);
            req_slice.write_val(0, &req).unwrap();
            req_slice.sync().unwrap();
            req_slice
        };
        let resp_slice = {
            let resp_slice = DmaStreamSlice::new(
                &self.control_response, 0, size_of::<VirtioGPURespResourceFlush>());
            resp_slice.write_val(0, &VirtioGPURespResourceFlush::default()).unwrap();
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
        let resp: VirtioGPURespSetScanout = resp_slice.read_val(0).unwrap();
        if resp.get_type() == VirtioGPUCtrlType::VIRTIO_GPU_RESP_OK_NODATA as u32 {
            Ok(())
        } else {
            Err(VirtioDeviceError::QueueUnknownError)
        }
    }
    pub fn update_cursor(&self, resource_id: u32, scanout_id: u32, pos_x: u32, pos_y: u32, hot_x: u32, hot_y: u32) -> Result<(), VirtioDeviceError> {
        let req_slice = {
            let req_slice = DmaStreamSlice::new(
                &self.cursor_request, 0, size_of::<VirtioGPUUpdateCursor>());
            let req = VirtioGPUUpdateCursor::new(VirtioGPUCursorPos::new(scanout_id, pos_x, pos_y), resource_id, hot_x, hot_y);
            req_slice.write_val(0, &req).unwrap();
            req_slice.sync().unwrap();
            req_slice
        };
        let resp_slice = {
            let resp_slice = DmaStreamSlice::new(
                &self.cursor_response, 0, size_of::<VirtioGPURespUpdateCursor>());
            resp_slice.write_val(0, &VirtioGPURespUpdateCursor::default()).unwrap();
            resp_slice.sync().unwrap();
            resp_slice
        };
        let mut queue = self.cursor_queue.disable_irq().lock();
        let _token = queue
            .add_dma_buf(&[&req_slice], &[&resp_slice])
            .expect("add queue failed");
        if queue.should_notify() {
            queue.notify();
        }
        while !queue.can_pop() {
            spin_loop();
        }
        queue.pop_used_with_token(_token).unwrap();
        Ok(())
    }
}
