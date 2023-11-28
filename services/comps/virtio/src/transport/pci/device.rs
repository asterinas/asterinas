use jinux_frame::{
    bus::{
        pci::{
            bus::PciDevice, capability::CapabilityData, common_device::PciCommonDevice, PciDeviceId,
        },
        BusProbeError,
    },
    io_mem::IoMem,
    offset_of,
    trap::IrqCallbackFunction,
    vm::DmaCoherent,
};

use alloc::{boxed::Box, sync::Arc};
use core::fmt::Debug;
use jinux_util::{field_ptr, safe_ptr::SafePtr};
use log::{info, warn};

use super::{common_cfg::VirtioPciCommonCfg, msix::VirtioMsixManager};
use crate::{
    queue::{AvailRing, Descriptor, UsedRing},
    transport::{
        pci::capability::{VirtioPciCapabilityData, VirtioPciCpabilityType},
        DeviceStatus, VirtioTransport, VirtioTransportError,
    },
    VirtioDeviceType,
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

pub struct VirtioPciTransport {
    device_type: VirtioDeviceType,
    common_device: PciCommonDevice,
    common_cfg: SafePtr<VirtioPciCommonCfg, IoMem>,
    device_cfg: VirtioPciCapabilityData,
    notify: VirtioPciNotify,
    msix_manager: VirtioMsixManager,
    device: Arc<VirtioPciDevice>,
}

impl PciDevice for VirtioPciDevice {
    fn device_id(&self) -> PciDeviceId {
        self.device_id
    }
}

impl Debug for VirtioPciTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PCIVirtioDevice")
            .field("common_device", &self.common_device)
            .finish()
    }
}

impl VirtioTransport for VirtioPciTransport {
    fn device_type(&self) -> VirtioDeviceType {
        self.device_type
    }

    fn set_queue(
        &mut self,
        idx: u16,
        queue_size: u16,
        descriptor_ptr: &SafePtr<Descriptor, DmaCoherent>,
        avail_ring_ptr: &SafePtr<AvailRing, DmaCoherent>,
        used_ring_ptr: &SafePtr<UsedRing, DmaCoherent>,
    ) -> Result<(), VirtioTransportError> {
        if idx >= self.num_queues() {
            return Err(VirtioTransportError::InvalidArgs);
        }
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_select)
            .write(&idx)
            .unwrap();
        debug_assert_eq!(
            field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_select)
                .read()
                .unwrap(),
            idx
        );

        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_size)
            .write(&queue_size)
            .unwrap();
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_desc)
            .write(&(descriptor_ptr.paddr() as u64))
            .unwrap();
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_driver)
            .write(&(avail_ring_ptr.paddr() as u64))
            .unwrap();
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_device)
            .write(&(used_ring_ptr.paddr() as u64))
            .unwrap();
        // Enable queue
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_enable)
            .write(&1u16)
            .unwrap();
        Ok(())
    }

    fn get_notify_ptr(&self, idx: u16) -> Result<SafePtr<u32, IoMem>, VirtioTransportError> {
        if idx >= self.num_queues() {
            return Err(VirtioTransportError::InvalidArgs);
        }
        Ok(SafePtr::new(
            self.notify.io_memory.clone(),
            (self.notify.offset + self.notify.offset_multiplier * idx as u32) as usize,
        ))
    }

    fn num_queues(&self) -> u16 {
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, num_queues)
            .read()
            .unwrap()
    }

    fn device_config_memory(&self) -> IoMem {
        let mut memory = self
            .device_cfg
            .memory_bar()
            .as_ref()
            .unwrap()
            .io_mem()
            .clone();
        let new_paddr = memory.paddr() + self.device_cfg.offset() as usize;
        memory
            .resize(new_paddr..(self.device_cfg.length() as usize + new_paddr))
            .unwrap();
        memory
    }

    fn device_features(&self) -> u64 {
        // select low
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, device_feature_select)
            .write(&0u32)
            .unwrap();
        let device_feature_low = field_ptr!(&self.common_cfg, VirtioPciCommonCfg, device_features)
            .read()
            .unwrap();
        // select high
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, device_feature_select)
            .write(&1u32)
            .unwrap();
        let device_feature_high = field_ptr!(&self.common_cfg, VirtioPciCommonCfg, device_features)
            .read()
            .unwrap() as u64;
        device_feature_high << 32 | device_feature_low as u64
    }

    fn set_driver_features(&mut self, features: u64) -> Result<(), VirtioTransportError> {
        let low = features as u32;
        let high = (features >> 32) as u32;
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, driver_feature_select)
            .write(&0u32)
            .unwrap();
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, driver_features)
            .write(&low)
            .unwrap();
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, driver_feature_select)
            .write(&1u32)
            .unwrap();
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, driver_features)
            .write(&high)
            .unwrap();
        Ok(())
    }

    fn device_status(&self) -> DeviceStatus {
        let status = field_ptr!(&self.common_cfg, VirtioPciCommonCfg, device_status)
            .read()
            .unwrap();
        DeviceStatus::from_bits(status).unwrap()
    }

    fn set_device_status(&mut self, status: DeviceStatus) -> Result<(), VirtioTransportError> {
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, device_status)
            .write(&(status.bits()))
            .unwrap();
        Ok(())
    }

    fn max_queue_size(&self, idx: u16) -> Result<u16, crate::transport::VirtioTransportError> {
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_select)
            .write(&idx)
            .unwrap();
        debug_assert_eq!(
            field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_select)
                .read()
                .unwrap(),
            idx
        );

        Ok(field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_size)
            .read()
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
        let (vector, irq) = if single_interrupt {
            self.msix_manager
                .pop_unused_irq()
                .ok_or(VirtioTransportError::NotEnoughResources)?
        } else {
            self.msix_manager.shared_interrupt_irq()
        };
        irq.on_active(func);
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_select)
            .write(&index)
            .unwrap();
        debug_assert_eq!(
            field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_select)
                .read()
                .unwrap(),
            index
        );
        field_ptr!(&self.common_cfg, VirtioPciCommonCfg, queue_msix_vector)
            .write(&vector)
            .unwrap();
        Ok(())
    }

    fn register_cfg_callback(
        &mut self,
        func: Box<IrqCallbackFunction>,
    ) -> Result<(), VirtioTransportError> {
        let (_, irq) = self.msix_manager.config_msix_irq();
        irq.on_active(func);
        Ok(())
    }

    fn is_legacy_version(&self) -> bool {
        // TODO: Support legacy version
        false
    }
}

impl VirtioPciTransport {
    pub(super) fn pci_device(&self) -> &Arc<VirtioPciDevice> {
        &self.device
    }

    #[allow(clippy::result_large_err)]
    pub(super) fn new(
        common_device: PciCommonDevice,
    ) -> Result<Self, (BusProbeError, PciCommonDevice)> {
        let device_type = match common_device.device_id().device_id {
            0x1000 => VirtioDeviceType::Network,
            0x1001 => VirtioDeviceType::Block,
            0x1002 => VirtioDeviceType::TraditionalMemoryBalloon,
            0x1003 => VirtioDeviceType::Console,
            0x1004 => VirtioDeviceType::ScsiHost,
            0x1005 => VirtioDeviceType::Entropy,
            0x1009 => VirtioDeviceType::Transport9P,
            id => {
                if id <= 0x1040 {
                    warn!(
                        "Unrecognized virtio-pci device id:{:x?}",
                        common_device.device_id().device_id
                    );
                    return Err((BusProbeError::ConfigurationSpaceError, common_device));
                }
                let id = id - 0x1040;
                match VirtioDeviceType::try_from(id as u8) {
                    Ok(device) => device,
                    Err(_) => {
                        warn!(
                            "Unrecognized virtio-pci device id:{:x?}",
                            common_device.device_id().device_id
                        );
                        return Err((BusProbeError::ConfigurationSpaceError, common_device));
                    }
                }
            }
        };

        info!("[Virtio]: Found device:{:?}", device_type);

        let mut msix = None;
        let mut notify = None;
        let mut common_cfg = None;
        let mut device_cfg = None;
        for cap in common_device.capabilities().iter() {
            match cap.capability_data() {
                CapabilityData::Vndr(vendor) => {
                    let data = VirtioPciCapabilityData::new(common_device.bar_manager(), *vendor);
                    match data.typ() {
                        VirtioPciCpabilityType::CommonCfg => {
                            common_cfg = Some(VirtioPciCommonCfg::new(&data));
                        }
                        VirtioPciCpabilityType::NotifyCfg => {
                            notify = Some(VirtioPciNotify {
                                offset_multiplier: data.option_value().unwrap(),
                                offset: data.offset(),
                                io_memory: data.memory_bar().as_ref().unwrap().io_mem().clone(),
                            });
                        }
                        VirtioPciCpabilityType::IsrCfg => {}
                        VirtioPciCpabilityType::DeviceCfg => {
                            device_cfg = Some(data);
                        }
                        VirtioPciCpabilityType::PciCfg => {}
                    }
                }
                CapabilityData::Msix(data) => {
                    msix = Some(data.clone());
                }
                CapabilityData::Unknown(id) => {
                    panic!("unknown capability: {}", id)
                }
                _ => {
                    panic!("PCI Virtio device should not have other type of capability")
                }
            }
        }
        // TODO: Support interrupt without MSI-X
        let msix = msix.unwrap();
        let notify = notify.unwrap();
        let common_cfg = common_cfg.unwrap();
        let device_cfg = device_cfg.unwrap();
        let msix_manager = VirtioMsixManager::new(msix);
        let device_id = *common_device.device_id();
        Ok(Self {
            common_device,
            common_cfg,
            device_cfg,
            notify,
            msix_manager,
            device_type,
            device: Arc::new(VirtioPciDevice { device_id }),
        })
    }
}
