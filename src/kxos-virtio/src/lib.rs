//! The virtio of kxos
#![no_std]
#![forbid(unsafe_code)]
#![allow(dead_code)]

extern crate alloc;
use alloc::{sync::Arc, vec::Vec};
use bitflags::bitflags;
use kxos_frame::{info, offset_of, TrapFrame};
use kxos_frame_pod_derive::Pod;
use kxos_pci::util::{PCIDevice, BAR};
use kxos_util::frame_ptr::InFramePtr;

use spin::{mutex::Mutex, MutexGuard};

use self::{block::VirtioBLKConfig, queue::VirtQueue};
use kxos_pci::{capability::vendor::virtio::CapabilityVirtioData, msix::MSIX};
#[macro_use]
extern crate kxos_frame_pod_derive;

pub mod block;
pub mod queue;

pub(crate) const PCI_VIRTIO_CAP_COMMON_CFG: u8 = 1;
pub(crate) const PCI_VIRTIO_CAP_NOTIFY_CFG: u8 = 2;
pub(crate) const PCI_VIRTIO_CAP_ISR_CFG: u8 = 3;
pub(crate) const PCI_VIRTIO_CAP_DEVICE_CFG: u8 = 4;
pub(crate) const PCI_VIRTIO_CAP_PCI_CFG: u8 = 5;

bitflags! {
    /// The device status field.
    pub struct DeviceStatus: u8 {
        /// Indicates that the guest OS has found the device and recognized it
        /// as a valid virtio device.
        const ACKNOWLEDGE = 1;

        /// Indicates that the guest OS knows how to drive the device.
        const DRIVER = 2;

        /// Indicates that something went wrong in the guest, and it has given
        /// up on the device. This could be an internal error, or the driver
        /// didn’t like the device for some reason, or even a fatal error
        /// during device operation.
        const FAILED = 128;

        /// Indicates that the driver has acknowledged all the features it
        /// understands, and feature negotiation is complete.
        const FEATURES_OK = 8;

        /// Indicates that the driver is set up and ready to drive the device.
        const DRIVER_OK = 4;

        /// Indicates that the device has experienced an error from which it
        /// can’t recover.
        const DEVICE_NEEDS_RESET = 64;
    }
}

#[derive(Debug, Default, Copy, Clone, Pod)]
#[repr(C)]
pub struct VitrioPciCommonCfg {
    device_feature_select: u32,
    device_feature: u32,
    driver_feature_select: u32,
    driver_feature: u32,
    config_msix_vector: u16,
    num_queues: u16,
    device_status: u8,
    config_generation: u8,

    queue_select: u16,
    queue_size: u16,
    queue_msix_vector: u16,
    queue_enable: u16,
    queue_notify_off: u16,
    queue_desc: u64,
    queue_driver: u64,
    queue_device: u64,
}

impl VitrioPciCommonCfg {
    pub(crate) fn new(cap: &CapabilityVirtioData, bars: [Option<BAR>; 6]) -> InFramePtr<Self> {
        let bar = cap.bar;
        let offset = cap.offset;
        match bars[bar as usize].expect("Virtio pci common cfg:bar is none") {
            BAR::Memory(address, _, _, _) => {
                info!("common_cfg addr:{:x}", (address as usize + offset as usize));
                InFramePtr::new(address as usize + offset as usize)
                    .expect("cannot get InFramePtr in VitioPciCommonCfg")
            }
            BAR::IO(_, _) => {
                panic!("Virtio pci common cfg:bar is IO type")
            }
        }
    }
}

#[derive(Debug)]
enum VirtioDeviceType {
    Network,
    Block,
    Console,
    Entropy,
    TraditionalMemoryBalloon,
    ScsiHost,
    GPU,
    Input,
    Crypto,
    Socket,
}
#[derive(Debug)]
enum VirtioDevice {
    Network,
    Block(InFramePtr<VirtioBLKConfig>),
    Console,
    Entropy,
    TraditionalMemoryBalloon,
    ScsiHost,
    GPU,
    Input,
    Crypto,
    Socket,
    Unknown,
}

pub struct PCIVirtioDevice {
    /// common config of one device
    common_cfg: InFramePtr<VitrioPciCommonCfg>,
    device: VirtioDevice,
    queues: Vec<Arc<Mutex<VirtQueue>>>,
    msix: MSIX,
}

impl PCIVirtioDevice {
    pub fn new(dev: Arc<PCIDevice>) -> Self {
        if dev.id.vendor_id != 0x1af4 {
            panic!("initialize PCIDevice failed, wrong PCI vendor id");
        }
        let device_type = match dev.id.device_id {
            0x1000 | 0x1041 => VirtioDeviceType::Network,
            0x1001 | 0x1042 => VirtioDeviceType::Block,
            0x1002 | 0x1043 => VirtioDeviceType::TraditionalMemoryBalloon,
            0x1003 | 0x1044 => VirtioDeviceType::Console,
            0x1004 | 0x1045 => VirtioDeviceType::ScsiHost,
            0x1005 | 0x1046 => VirtioDeviceType::Entropy,
            // 0x1009 | 0x104a => VirtioDeviceType::,
            _ => {
                panic!("initialize PCIDevice failed, unrecognized Virtio Device Type")
            }
        };
        let bars = dev.bars;
        let loc = dev.loc;
        let mut notify_base_address = 0;
        let mut notify_off_multiplier = 0;
        let mut device = VirtioDevice::Unknown;
        let mut msix = MSIX::default();
        let mut common_cfg_frame_ptr_some = None;
        for cap in dev.capabilities.iter() {
            match &cap.data {
                kxos_pci::capability::CapabilityData::VNDR(vndr_data) => match vndr_data {
                    kxos_pci::capability::vendor::CapabilityVNDRData::VIRTIO(cap_data) => {
                        match cap_data.cfg_type {
                            PCI_VIRTIO_CAP_COMMON_CFG => {
                                common_cfg_frame_ptr_some =
                                    Some(VitrioPciCommonCfg::new(cap_data, bars));
                            }
                            PCI_VIRTIO_CAP_NOTIFY_CFG => {
                                notify_off_multiplier = cap_data.option.unwrap();
                                match bars[cap_data.bar as usize]
                                    .expect("initialize PCIDevice failed, notify bar is None")
                                {
                                    BAR::Memory(address, _, _, _) => {
                                        notify_base_address = address + cap_data.offset as u64;
                                    }
                                    BAR::IO(_, _) => {
                                        panic!("initialize PCIDevice failed, notify bar is IO Type")
                                    }
                                };
                            }
                            PCI_VIRTIO_CAP_ISR_CFG => {}
                            PCI_VIRTIO_CAP_DEVICE_CFG => {
                                device = match device_type {
                                    VirtioDeviceType::Block => {
                                        VirtioDevice::Block(VirtioBLKConfig::new(&cap_data, bars))
                                    }
                                    _ => {
                                        panic!(
                                                "initialize PCIDevice failed, unsupport Virtio Device Type"
                                            )
                                    }
                                }
                            }
                            PCI_VIRTIO_CAP_PCI_CFG => {}
                            _ => panic!("unsupport cfg, cfg_type:{}", cap_data.cfg_type),
                        };
                    }
                },

                kxos_pci::capability::CapabilityData::MSIX(cap_data) => {
                    msix = MSIX::new(&cap_data, bars, loc, cap.cap_ptr);
                }
                kxos_pci::capability::CapabilityData::Unknown(id) => {
                    panic!("unknown capability device:{}", id)
                }
                _ => {
                    panic!("PCI Virtio device should not have other type of capability")
                }
            }
        }
        let common_cfg_frame_ptr = if common_cfg_frame_ptr_some.is_none() {
            panic!("Vitio Common cfg is None")
        } else {
            common_cfg_frame_ptr_some.unwrap()
        };
        info!(
            "common_cfg_num_queues:{:x}",
            common_cfg_frame_ptr.read_at(offset_of!(VitrioPciCommonCfg, num_queues))
        );
        // let b : InFramePtr<u8> = InFramePtr::new(common_cfg_frame_ptr.paddr()+19).expect("test");
        // info!("test_Aaaaaa:{:#x?}",b.read());
        if msix.table_size
            != common_cfg_frame_ptr.read_at(offset_of!(VitrioPciCommonCfg, num_queues)) as u16
        {
            panic!("the msix table size is not match with the number of queues");
        }
        let mut queues = Vec::new();
        common_cfg_frame_ptr.write_at(
            offset_of!(VitrioPciCommonCfg, device_status),
            DeviceStatus::ACKNOWLEDGE.bits(),
        );
        common_cfg_frame_ptr.write_at(
            offset_of!(VitrioPciCommonCfg, device_status),
            DeviceStatus::DRIVER.bits(),
        );
        common_cfg_frame_ptr.write_at(
            offset_of!(VitrioPciCommonCfg, device_status),
            DeviceStatus::FEATURES_OK.bits(),
        );
        for i in 0..common_cfg_frame_ptr.read_at(offset_of!(VitrioPciCommonCfg, num_queues)) as u16
        {
            queues.push(Arc::new(Mutex::new(
                VirtQueue::new(
                    &common_cfg_frame_ptr,
                    i as usize,
                    16,
                    notify_base_address as usize,
                    notify_off_multiplier,
                    i,
                )
                .expect("create virtqueue failed"),
            )));
        }
        common_cfg_frame_ptr.write_at(
            offset_of!(VitrioPciCommonCfg, device_status),
            DeviceStatus::DRIVER_OK.bits(),
        );
        Self {
            common_cfg: common_cfg_frame_ptr,
            device,
            queues,
            msix,
        }
    }

    pub fn get_queue(&self, queue_index: u16) -> MutexGuard<VirtQueue> {
        self.queues
            .get(queue_index as usize)
            .expect("index out of range")
            .lock()
    }

    /// register the queue interrupt functions, this function should call only once
    pub fn register_queue_interrupt_functions<F>(&mut self, functions: &mut Vec<F>)
    where
        F: Fn(&TrapFrame) + Send + Sync + 'static,
    {
        let len = functions.len();
        if len
            != self
                .common_cfg
                .read_at(offset_of!(VitrioPciCommonCfg, num_queues)) as usize
        {
            panic!("the size of queue interrupt functions not equal to the number of queues, functions amount:{}, queues amount:{}",len,
            self.common_cfg.read_at(offset_of!(VitrioPciCommonCfg,num_queues)));
        }

        functions.reverse();
        for i in 0..len {
            let function = functions.pop().unwrap();
            let msix = self.msix.table.get_mut(i).unwrap();
            if !msix.irq_handle.is_empty() {
                panic!("function `register_queue_interrupt_functions` called more than one time");
            }
            msix.irq_handle.on_active(function);
        }
    }

    fn check_subset(smaller: u32, bigger: u32) -> bool {
        let mut temp: u32 = 1;
        for _ in 0..31 {
            if (smaller & temp) > (bigger & temp) {
                return false;
            }
            temp <<= 1;
        }
        if (smaller & temp) > (bigger & temp) {
            false
        } else {
            true
        }
    }
}
