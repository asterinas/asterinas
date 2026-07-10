// SPDX-License-Identifier: MPL-2.0

#[cfg(target_arch = "riscv64")]
use alloc::vec::Vec;
use alloc::{boxed::Box, sync::Arc};
use core::fmt::Debug;

use aster_pci::{
    PciDeviceId, bus::PciDevice, cfg_space::BarAccess, common_device::PciCommonDevice,
};
use aster_util::{field_ptr, safe_ptr::SafePtr};
#[cfg(target_arch = "riscv64")]
use ostd::{arch::trap::TrapFrame, mm::VmIoOnce, sync::RwLock};
use ostd::{
    bus::BusProbeError,
    info,
    io::IoMem,
    irq::IrqCallbackFunction,
    mm::{HasDaddr, dma::DmaCoherent},
    warn,
};

use super::{common_cfg::VirtioPciCommonCfg, msix::VirtioMsixManager};
use crate::{
    VirtioDeviceType,
    queue::{AvailRing, Descriptor, UsedRing},
    transport::{
        ConfigManager, DeviceStatus, VirtioTransport, VirtioTransportError,
        pci::capability::{VirtioPciCapabilityData, VirtioPciCpabilityType},
    },
};

pub struct VirtioPciNotify {
    offset_multiplier: u32,
    offset: u32,
    io_memory: IoMem,
}

#[derive(Debug)]
pub struct VirtioPciDevice {
    device_id: PciDeviceId,
}

impl VirtioPciDevice {
    pub(super) fn new(device_id: PciDeviceId) -> Self {
        Self { device_id }
    }
}

impl PciDevice for VirtioPciDevice {
    fn device_id(&self) -> PciDeviceId {
        self.device_id
    }
}

pub struct VirtioPciModernTransport {
    device_type: VirtioDeviceType,
    common_device: PciCommonDevice,
    common_cfg: SafePtr<VirtioPciCommonCfg, IoMem>,
    device_cfg: VirtioPciCapabilityData,
    notify: VirtioPciNotify,
    interrupt: VirtioPciInterrupt,
}

enum VirtioPciInterrupt {
    Msix(VirtioMsixManager),
    #[cfg(target_arch = "riscv64")]
    Intx(Arc<RwLock<VirtioPciIntxManager>>),
}

#[cfg(target_arch = "riscv64")]
struct VirtioPciIntxManager {
    irq: aster_pci::common_device::MappedPciIrqLine,
    queue_callbacks: Vec<Box<IrqCallbackFunction>>,
    cfg_callbacks: Vec<Box<IrqCallbackFunction>>,
    isr_io_memory: IoMem,
    isr_offset: usize,
}

#[cfg(target_arch = "riscv64")]
impl VirtioPciIntxManager {
    fn new(
        irq: aster_pci::common_device::MappedPciIrqLine,
        isr_cfg: VirtioPciCapabilityData,
    ) -> Arc<RwLock<Self>> {
        let intx = Arc::new(RwLock::new(Self {
            irq,
            queue_callbacks: Vec::new(),
            cfg_callbacks: Vec::new(),
            isr_io_memory: isr_cfg.memory_bar().unwrap().clone(),
            isr_offset: isr_cfg.offset() as usize,
        }));

        let weak = Arc::downgrade(&intx);
        let callback = move |trap_frame: &TrapFrame| {
            let Some(intx) = weak.upgrade() else {
                return;
            };
            let intx = intx.read();
            let interrupt_status = intx.isr_io_memory.read_once::<u8>(intx.isr_offset).unwrap();
            if interrupt_status & 0x01 != 0 {
                for callback in intx.queue_callbacks.iter() {
                    callback(trap_frame);
                }
            }
            if interrupt_status & 0x02 != 0 {
                for callback in intx.cfg_callbacks.iter() {
                    callback(trap_frame);
                }
            }
        };

        intx.write().irq.on_active(callback);
        intx
    }

    fn register_queue_callback(&mut self, func: Box<IrqCallbackFunction>) {
        self.queue_callbacks.push(func);
    }

    fn register_cfg_callback(&mut self, func: Box<IrqCallbackFunction>) {
        self.cfg_callbacks.push(func);
    }
}

impl Debug for VirtioPciModernTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PCIVirtioModernDevice")
            .field("common_device", &self.common_device)
            .finish()
    }
}

impl VirtioTransport for VirtioPciModernTransport {
    fn device_type(&self) -> VirtioDeviceType {
        self.device_type
    }

    fn set_queue(
        &mut self,
        idx: u16,
        queue_size: u16,
        descriptor_ptr: &SafePtr<Descriptor, Arc<DmaCoherent>>,
        avail_ring_ptr: &SafePtr<AvailRing, Arc<DmaCoherent>>,
        used_ring_ptr: &SafePtr<UsedRing, Arc<DmaCoherent>>,
    ) -> Result<(), VirtioTransportError> {
        if idx >= self.num_queues() {
            return Err(VirtioTransportError::InvalidArgs);
        }
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_select)
            .write_once(&idx)
            .unwrap();
        debug_assert_eq!(
            field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_select)
                .read_once()
                .unwrap(),
            idx
        );

        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_size)
            .write_once(&queue_size)
            .unwrap();
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_desc)
            .write_once(&(descriptor_ptr.daddr() as u64))
            .unwrap();
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_driver)
            .write_once(&(avail_ring_ptr.daddr() as u64))
            .unwrap();
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_device)
            .write_once(&(used_ring_ptr.daddr() as u64))
            .unwrap();
        // Enable queue
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_enable)
            .write_once(&1u16)
            .unwrap();
        Ok(())
    }

    fn notify_config(&self, idx: usize) -> ConfigManager<u32> {
        debug_assert!(idx < self.num_queues() as usize);
        let safe_ptr = Some(SafePtr::new(
            self.notify.io_memory.clone(),
            (self.notify.offset + self.notify.offset_multiplier * idx as u32) as usize,
        ));

        ConfigManager::new(safe_ptr, None)
    }

    fn num_queues(&self) -> u16 {
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, num_queues)
            .read_once()
            .unwrap()
    }

    fn device_config_mem(&self) -> Option<IoMem> {
        let offset = self.device_cfg.offset() as usize;
        let length = self.device_cfg.length() as usize;
        let io_mem = self
            .device_cfg
            .memory_bar()
            .unwrap()
            .slice(offset..offset + length);

        Some(io_mem)
    }

    fn device_config_bar(&self) -> Option<(BarAccess, usize)> {
        None
    }

    fn read_device_features(&self) -> u64 {
        // select low
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, device_feature_select)
            .write_once(&0u32)
            .unwrap();
        let device_feature_low = field_ptr!(&self.common_cfg, VirtioPciCommonCfg, device_features)
            .read_once()
            .unwrap();
        // select high
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, device_feature_select)
            .write_once(&1u32)
            .unwrap();
        let device_feature_high = field_ptr!(&self.common_cfg, VirtioPciCommonCfg, device_features)
            .read_once()
            .unwrap() as u64;
        (device_feature_high << 32) | device_feature_low as u64
    }

    fn write_driver_features(&mut self, features: u64) -> Result<(), VirtioTransportError> {
        let low = features as u32;
        let high = (features >> 32) as u32;
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, driver_feature_select)
            .write_once(&0u32)
            .unwrap();
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, driver_features)
            .write_once(&low)
            .unwrap();
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, driver_feature_select)
            .write_once(&1u32)
            .unwrap();
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, driver_features)
            .write_once(&high)
            .unwrap();
        Ok(())
    }

    fn read_device_status(&self) -> DeviceStatus {
        let status = field_ptr!(&self.common_cfg, VirtioPciCommonCfg, device_status)
            .read_once()
            .unwrap();
        DeviceStatus::from_bits(status).unwrap()
    }

    fn write_device_status(&mut self, status: DeviceStatus) -> Result<(), VirtioTransportError> {
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, device_status)
            .write_once(&(status.bits()))
            .unwrap();
        Ok(())
    }

    fn max_queue_size(&self, idx: u16) -> Result<u16, VirtioTransportError> {
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_select)
            .write_once(&idx)
            .unwrap();
        debug_assert_eq!(
            field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_select)
                .read_once()
                .unwrap(),
            idx
        );

        Ok(field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_size)
            .read_once()
            .unwrap())
    }

    fn register_queue_callback(
        &mut self,
        index: u16,
        func: Box<IrqCallbackFunction>,
        single_interrupt: bool,
    ) -> Result<(), VirtioTransportError> {
        if index >= self.num_queues() {
            return Err(VirtioTransportError::InvalidArgs);
        }
        let device_type = self.device_type;
        match &mut self.interrupt {
            VirtioPciInterrupt::Msix(msix_manager) => {
                let (vector, irq) = if single_interrupt {
                    if let Some(unused_irq) = msix_manager.pop_unused_irq() {
                        unused_irq
                    } else {
                        warn!(
                            "{:?}: `single_interrupt` ignored: no more IRQ lines available",
                            device_type
                        );
                        msix_manager.shared_irq_line()
                    }
                } else {
                    msix_manager.shared_irq_line()
                };
                irq.on_active(func);
                field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_select)
                    .write_once(&index)
                    .unwrap();
                debug_assert_eq!(
                    field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_select)
                        .read_once()
                        .unwrap(),
                    index
                );
                field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_msix_vector)
                    .write_once(&vector)
                    .unwrap();
            }
            #[cfg(target_arch = "riscv64")]
            VirtioPciInterrupt::Intx(intx_manager) => {
                intx_manager.write().register_queue_callback(func);
            }
        }
        Ok(())
    }

    fn register_cfg_callback(
        &mut self,
        func: Box<IrqCallbackFunction>,
    ) -> Result<(), VirtioTransportError> {
        match &mut self.interrupt {
            VirtioPciInterrupt::Msix(msix_manager) => {
                let (_, irq) = msix_manager.config_msix_irq();
                irq.on_active(func);
            }
            #[cfg(target_arch = "riscv64")]
            VirtioPciInterrupt::Intx(intx_manager) => {
                intx_manager.write().register_cfg_callback(func);
            }
        }
        Ok(())
    }

    fn is_legacy_version(&self) -> bool {
        // TODO: Support legacy version
        false
    }
}

impl VirtioPciModernTransport {
    #[expect(clippy::result_large_err)]
    pub(super) fn new(
        mut common_device: PciCommonDevice,
    ) -> Result<Self, (BusProbeError, PciCommonDevice)> {
        let device_id = common_device.device_id().device_id;
        let device_type_value = if device_id <= 0x1040 {
            device_id - 0x1000
        } else {
            device_id - 0x1040
        };

        let device_type = match VirtioDeviceType::try_from(device_type_value as u8) {
            Ok(device) => device,
            Err(_) => {
                warn!("Unrecognized virtio-pci device ID: {:x?}", device_id);
                return Err((BusProbeError::DeviceNotMatch, common_device));
            }
        };

        info!("Found device: {:?}", device_type);

        let mut notify = None;
        let mut common_cfg = None;
        let mut device_cfg = None;
        let mut isr_cfg = None;
        let (vndr_caps, bar_manager) = common_device.iter_vndr_capability_with_bar_manager();
        for vndr_cap in vndr_caps {
            let data = VirtioPciCapabilityData::new(bar_manager, vndr_cap);
            match data.typ() {
                VirtioPciCpabilityType::CommonCfg => {
                    common_cfg = Some(VirtioPciCommonCfg::new(&data));
                }
                VirtioPciCpabilityType::NotifyCfg => {
                    notify = Some(VirtioPciNotify {
                        offset_multiplier: data.option_value().unwrap(),
                        offset: data.offset(),
                        io_memory: data.memory_bar().unwrap().clone(),
                    });
                }
                VirtioPciCpabilityType::IsrCfg => {
                    isr_cfg = Some(data);
                }
                VirtioPciCpabilityType::DeviceCfg => {
                    device_cfg = Some(data);
                }
                VirtioPciCpabilityType::PciCfg => {}
            }
        }
        let notify = notify.unwrap();
        let common_cfg = common_cfg.unwrap();
        let device_cfg = device_cfg.unwrap();

        let Ok(interrupt) = create_interrupt_manager(&mut common_device, isr_cfg) else {
            return Err((BusProbeError::ConfigurationSpaceError, common_device));
        };

        Ok(Self {
            device_type,
            common_device,
            common_cfg,
            device_cfg,
            notify,
            interrupt,
        })
    }
}

fn create_interrupt_manager(
    common_device: &mut PciCommonDevice,
    #[cfg_attr(not(target_arch = "riscv64"), expect(unused_variables))] isr_cfg: Option<
        VirtioPciCapabilityData,
    >,
) -> Result<VirtioPciInterrupt, VirtioTransportError> {
    #[cfg(target_arch = "riscv64")]
    {
        if let Some(isr_cfg) = isr_cfg
            && let Ok(mapped_irq) = common_device.map_intx_interrupt()
        {
            common_device.enable_intx_interrupt();
            return Ok(VirtioPciInterrupt::Intx(VirtioPciIntxManager::new(
                mapped_irq, isr_cfg,
            )));
        }
    }

    if let Ok(Some(msix)) = common_device.acquire_msix_capability() {
        return Ok(VirtioPciInterrupt::Msix(VirtioMsixManager::new(msix)));
    }

    Err(VirtioTransportError::DeviceStatusError)
}
