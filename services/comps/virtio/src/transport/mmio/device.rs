// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc};
use aster_frame::{
    bus::mmio::{
        bus::MmioDevice,
        device::{MmioCommonDevice, VirtioMmioVersion},
    },
    config::PAGE_SIZE,
    io_mem::IoMem,
    offset_of,
    sync::RwLock,
    trap::IrqCallbackFunction,
    vm::DmaCoherent,
};
use aster_rights::{ReadOp, WriteOp};
use aster_util::{field_ptr, safe_ptr::SafePtr};
use core::mem::size_of;
use log::warn;

use crate::{
    queue::{AvailRing, Descriptor, UsedRing},
    transport::{DeviceStatus, VirtioTransport, VirtioTransportError},
    VirtioDeviceType,
};

use super::{layout::VirtioMmioLayout, multiplex::MultiplexIrq};

#[derive(Debug)]
pub struct VirtioMmioDevice {
    device_id: u32,
}

#[derive(Debug)]
pub struct VirtioMmioTransport {
    layout: SafePtr<VirtioMmioLayout, IoMem>,
    device: Arc<VirtioMmioDevice>,
    common_device: aster_frame::bus::mmio::device::MmioCommonDevice,
    multiplex: Arc<RwLock<MultiplexIrq>>,
}

impl MmioDevice for VirtioMmioDevice {
    fn device_id(&self) -> u32 {
        self.device_id
    }
}

impl MmioDevice for VirtioMmioTransport {
    fn device_id(&self) -> u32 {
        self.common_device.device_id()
    }
}

impl VirtioMmioTransport {
    pub(super) fn mmio_device(&self) -> &Arc<VirtioMmioDevice> {
        &self.device
    }

    pub(super) fn new(device: MmioCommonDevice) -> Self {
        let irq = device.irq().clone();
        let layout = SafePtr::new(device.io_mem().clone(), 0);
        let device_id = device.device_id();
        let (interrupt_ack, interrupt_status) = {
            let interrupt_ack_offset = offset_of!(VirtioMmioLayout, interrupt_ack);
            let interrupt_status_offset = offset_of!(VirtioMmioLayout, interrupt_status);
            let mut interrupt_ack = layout.clone();
            interrupt_ack.byte_add(interrupt_ack_offset as usize);
            let mut interrupt_status = layout.clone();
            interrupt_status.byte_add(interrupt_status_offset as usize);
            (
                interrupt_ack.cast::<u32>().restrict::<WriteOp>(),
                interrupt_status.cast::<u32>().restrict::<ReadOp>(),
            )
        };
        let device = Self {
            layout,
            common_device: device,
            multiplex: MultiplexIrq::new(irq, interrupt_ack, interrupt_status),
            device: Arc::new(VirtioMmioDevice { device_id }),
        };
        if device.common_device.version() == VirtioMmioVersion::Legacy {
            field_ptr!(&device.layout, VirtioMmioLayout, legacy_guest_page_size)
                .write(&(PAGE_SIZE as u32))
                .unwrap();
        }
        device
    }
}

impl VirtioTransport for VirtioMmioTransport {
    fn device_type(&self) -> VirtioDeviceType {
        VirtioDeviceType::try_from(self.common_device.device_id() as u8).unwrap()
    }

    fn set_queue(
        &mut self,
        idx: u16,
        queue_size: u16,
        descriptor_ptr: &SafePtr<Descriptor, DmaCoherent>,
        driver_ptr: &SafePtr<AvailRing, DmaCoherent>,
        device_ptr: &SafePtr<UsedRing, DmaCoherent>,
    ) -> Result<(), VirtioTransportError> {
        field_ptr!(&self.layout, VirtioMmioLayout, queue_sel)
            .write(&(idx as u32))
            .unwrap();

        let queue_num_max: u32 = field_ptr!(&self.layout, VirtioMmioLayout, queue_num_max)
            .read()
            .unwrap();

        if queue_size as u32 > queue_num_max {
            warn!("Set queue failed, queue size is bigger than maximum virtual queue size.");
            return Err(VirtioTransportError::InvalidArgs);
        }

        let descriptor_paddr = descriptor_ptr.paddr();
        let driver_paddr = driver_ptr.paddr();
        let device_paddr = device_ptr.paddr();

        field_ptr!(&self.layout, VirtioMmioLayout, queue_num)
            .write(&(queue_size as u32))
            .unwrap();

        match self.common_device.version() {
            VirtioMmioVersion::Legacy => {
                // The area should be continuous
                assert_eq!(
                    driver_paddr - descriptor_paddr,
                    size_of::<Descriptor>() * queue_size as usize
                );
                // Descriptor paddr should align
                assert_eq!(descriptor_paddr % PAGE_SIZE, 0);
                let pfn = (descriptor_paddr / PAGE_SIZE) as u32;
                field_ptr!(&self.layout, VirtioMmioLayout, legacy_queue_align)
                    .write(&(PAGE_SIZE as u32))
                    .unwrap();
                field_ptr!(&self.layout, VirtioMmioLayout, legacy_queue_pfn)
                    .write(&pfn)
                    .unwrap();
            }
            VirtioMmioVersion::Modern => {
                field_ptr!(&self.layout, VirtioMmioLayout, queue_desc_low)
                    .write(&(descriptor_paddr as u32))
                    .unwrap();
                field_ptr!(&self.layout, VirtioMmioLayout, queue_desc_high)
                    .write(&((descriptor_paddr >> 32) as u32))
                    .unwrap();

                field_ptr!(&self.layout, VirtioMmioLayout, queue_driver_low)
                    .write(&(driver_paddr as u32))
                    .unwrap();
                field_ptr!(&self.layout, VirtioMmioLayout, queue_driver_high)
                    .write(&((driver_paddr >> 32) as u32))
                    .unwrap();

                field_ptr!(&self.layout, VirtioMmioLayout, queue_device_low)
                    .write(&(device_paddr as u32))
                    .unwrap();
                field_ptr!(&self.layout, VirtioMmioLayout, queue_device_high)
                    .write(&((device_paddr >> 32) as u32))
                    .unwrap();
                // enable queue
                field_ptr!(&self.layout, VirtioMmioLayout, queue_sel)
                    .write(&(idx as u32))
                    .unwrap();
                field_ptr!(&self.layout, VirtioMmioLayout, queue_ready)
                    .write(&1u32)
                    .unwrap();
            }
        };
        Ok(())
    }

    fn get_notify_ptr(&self, _idx: u16) -> Result<SafePtr<u32, IoMem>, VirtioTransportError> {
        let offset = offset_of!(VirtioMmioLayout, queue_notify) as usize;
        Ok(SafePtr::new(self.common_device.io_mem().clone(), offset))
    }

    fn num_queues(&self) -> u16 {
        // We use the field `queue_num_max` to get queue size.
        // If the queue is not exists, the field should be zero
        let mut num_queues = 0;
        const MAX_QUEUES: u32 = 512;
        while num_queues < MAX_QUEUES {
            field_ptr!(&self.layout, VirtioMmioLayout, queue_sel)
                .write(&num_queues)
                .unwrap();
            if field_ptr!(&self.layout, VirtioMmioLayout, queue_num_max)
                .read()
                .unwrap()
                == 0u32
            {
                return num_queues as u16;
            }
            num_queues += 1;
        }
        todo!()
    }

    fn device_config_memory(&self) -> IoMem {
        // offset: 0x100~0x200
        let mut io_mem = self.common_device.io_mem().clone();
        let paddr = io_mem.paddr();
        io_mem.resize((paddr + 0x100)..(paddr + 0x200)).unwrap();
        io_mem
    }

    fn device_features(&self) -> u64 {
        // select low
        field_ptr!(&self.layout, VirtioMmioLayout, device_features_select)
            .write(&0u32)
            .unwrap();
        let device_feature_low = field_ptr!(&self.layout, VirtioMmioLayout, device_features)
            .read()
            .unwrap();
        // select high
        field_ptr!(&self.layout, VirtioMmioLayout, device_features_select)
            .write(&1u32)
            .unwrap();
        let device_feature_high = field_ptr!(&self.layout, VirtioMmioLayout, device_features)
            .read()
            .unwrap() as u64;
        device_feature_high << 32 | device_feature_low as u64
    }

    fn set_driver_features(&mut self, features: u64) -> Result<(), VirtioTransportError> {
        let low = features as u32;
        let high = (features >> 32) as u32;
        field_ptr!(&self.layout, VirtioMmioLayout, driver_features_select)
            .write(&0u32)
            .unwrap();
        field_ptr!(&self.layout, VirtioMmioLayout, driver_features)
            .write(&low)
            .unwrap();
        field_ptr!(&self.layout, VirtioMmioLayout, driver_features_select)
            .write(&1u32)
            .unwrap();
        field_ptr!(&self.layout, VirtioMmioLayout, driver_features)
            .write(&high)
            .unwrap();
        Ok(())
    }

    fn device_status(&self) -> DeviceStatus {
        DeviceStatus::from_bits(
            field_ptr!(&self.layout, VirtioMmioLayout, status)
                .read()
                .unwrap() as u8,
        )
        .unwrap()
    }

    fn set_device_status(&mut self, status: DeviceStatus) -> Result<(), VirtioTransportError> {
        field_ptr!(&self.layout, VirtioMmioLayout, status)
            .write(&(status.bits() as u32))
            .unwrap();
        Ok(())
    }

    fn is_legacy_version(&self) -> bool {
        self.common_device.version() == VirtioMmioVersion::Legacy
    }

    fn max_queue_size(&self, idx: u16) -> Result<u16, VirtioTransportError> {
        field_ptr!(&self.layout, VirtioMmioLayout, queue_sel)
            .write(&(idx as u32))
            .unwrap();
        Ok(field_ptr!(&self.layout, VirtioMmioLayout, queue_num_max)
            .read()
            .unwrap() as u16)
    }

    fn register_queue_callback(
        &mut self,
        _index: u16,
        func: Box<IrqCallbackFunction>,
        single_interrupt: bool,
    ) -> Result<(), VirtioTransportError> {
        if single_interrupt {
            return Err(VirtioTransportError::NotEnoughResources);
        }
        self.multiplex.write().register_queue_callback(func);
        Ok(())
    }

    fn register_cfg_callback(
        &mut self,
        func: Box<IrqCallbackFunction>,
    ) -> Result<(), VirtioTransportError> {
        self.multiplex.write().register_cfg_callback(func);
        Ok(())
    }
}
