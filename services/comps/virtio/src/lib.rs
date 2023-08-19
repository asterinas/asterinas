//! The virtio of jinux
#![no_std]
#![forbid(unsafe_code)]
#![allow(dead_code)]

extern crate alloc;

use component::init_component;
use core::{mem::size_of, str::FromStr};

use alloc::{collections::VecDeque, string::String, sync::Arc, vec::Vec};
use bitflags::bitflags;
use component::ComponentInitError;
use device::VirtioDevice;
use jinux_frame::sync::Mutex;
use jinux_frame::{io_mem::IoMem, offset_of, trap::TrapFrame};
use jinux_pci::{util::BAR, PciDevice};
use jinux_util::{field_ptr, safe_ptr::SafePtr};
use log::{debug, info};
use pod::Pod;
use spin::Once;

use crate::device::VirtioInfo;
use jinux_pci::{capability::vendor::virtio::CapabilityVirtioData, msix::MSIX};

pub mod device;
pub mod queue;

pub static VIRTIO_COMPONENT: Once<VIRTIOComponent> = Once::new();

#[init_component]
fn virtio_component_init() -> Result<(), ComponentInitError> {
    let a = VIRTIOComponent::init()?;
    VIRTIO_COMPONENT.call_once(|| a);
    Ok(())
}

pub struct VIRTIOComponent {
    virtio_devices: Mutex<VecDeque<PCIVirtioDevice>>,
}

impl VIRTIOComponent {
    pub fn init() -> Result<Self, ComponentInitError> {
        let pci_devices =
            jinux_pci::PCI_COMPONENT
                .get()
                .ok_or(ComponentInitError::UninitializedDependencies(
                    String::from_str("PCI").unwrap(),
                ))?;
        let mut virtio_devices = VecDeque::new();
        for index in 0..pci_devices.device_amount() {
            let pci_device = pci_devices.get_pci_devices(index).unwrap();
            if pci_device.id.vendor_id == 0x1af4 {
                virtio_devices.push_back(PCIVirtioDevice::new(pci_device));
            }
        }
        Ok(Self {
            virtio_devices: Mutex::new(virtio_devices),
        })
    }

    pub const fn name() -> &'static str {
        "Virtio"
    }
    // 0~65535
    pub const fn priority() -> u16 {
        256
    }
}

impl VIRTIOComponent {
    pub fn pop(self: &Self) -> Option<PCIVirtioDevice> {
        self.virtio_devices.lock().pop_front()
    }

    pub fn get_device(self: &Self, device_type: VirtioDeviceType) -> Vec<PCIVirtioDevice> {
        let mut devices = Vec::new();
        let mut lock = self.virtio_devices.lock();
        let len = lock.len();
        for _ in 0..len {
            let device = lock.pop_front().unwrap();
            let d_type = VirtioDeviceType::from_virtio_device(&device.device);
            if d_type == device_type {
                devices.push(device);
            } else {
                lock.push_back(device);
            }
        }
        devices
    }
}

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

bitflags! {
    /// all device features, bits 0~23 and 50~63 are sepecified by device.
    /// if using this struct to translate u64, use from_bits_truncate function instead of from_bits
    ///
    struct Feature: u64 {

        // device independent
        const NOTIFY_ON_EMPTY       = 1 << 24; // legacy
        const ANY_LAYOUT            = 1 << 27; // legacy
        const RING_INDIRECT_DESC    = 1 << 28;
        const RING_EVENT_IDX        = 1 << 29;
        const UNUSED                = 1 << 30; // legacy
        const VERSION_1             = 1 << 32; // detect legacy

        // since virtio v1.1
        const ACCESS_PLATFORM       = 1 << 33;
        const RING_PACKED           = 1 << 34;
        const IN_ORDER              = 1 << 35;
        const ORDER_PLATFORM        = 1 << 36;
        const SR_IOV                = 1 << 37;
        const NOTIFICATION_DATA     = 1 << 38;
    }
}

#[derive(Debug, Default, Copy, Clone, Pod)]
#[repr(C)]
pub struct VirtioPciCommonCfg {
    device_feature_select: u32,
    device_feature: u32,
    driver_feature_select: u32,
    driver_feature: u32,
    pub config_msix_vector: u16,
    num_queues: u16,
    pub device_status: u8,
    config_generation: u8,

    queue_select: u16,
    queue_size: u16,
    pub queue_msix_vector: u16,
    queue_enable: u16,
    queue_notify_off: u16,
    queue_desc: u64,
    queue_driver: u64,
    queue_device: u64,
}

impl VirtioPciCommonCfg {
    pub(crate) fn new(cap: &CapabilityVirtioData, bars: [Option<BAR>; 6]) -> SafePtr<Self, IoMem> {
        let bar = cap.bar;
        let offset = cap.offset;
        match bars[bar as usize].expect("Virtio pci common cfg:bar is none") {
            BAR::Memory(address, _, _, _) => {
                debug!("common_cfg addr:{:x}", (address as usize + offset as usize));
                SafePtr::new(
                    IoMem::new(
                        (address as usize + offset as usize)
                            ..(address as usize + offset as usize + size_of::<Self>()),
                    )
                    .unwrap(),
                    0,
                )
            }
            BAR::IO(first, second) => {
                panic!(
                    "Virtio pci common cfg:bar is IO type, value:{:x}, {:x}",
                    first, second
                )
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum VirtioDeviceType {
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
    Unknown,
}

impl VirtioDeviceType {
    pub fn from_virtio_device(device: &VirtioDevice) -> Self {
        match device {
            VirtioDevice::Network(_) => VirtioDeviceType::Network,
            VirtioDevice::Block(_) => VirtioDeviceType::Block,
            VirtioDevice::Console => VirtioDeviceType::Console,
            VirtioDevice::Entropy => VirtioDeviceType::Entropy,
            VirtioDevice::TraditionalMemoryBalloon => VirtioDeviceType::TraditionalMemoryBalloon,
            VirtioDevice::ScsiHost => VirtioDeviceType::ScsiHost,
            VirtioDevice::GPU => VirtioDeviceType::GPU,
            VirtioDevice::Input(_) => VirtioDeviceType::Input,
            VirtioDevice::Crypto => VirtioDeviceType::Crypto,
            VirtioDevice::Socket => VirtioDeviceType::Socket,
            VirtioDevice::Unknown => VirtioDeviceType::Unknown,
        }
    }
}

pub struct PCIVirtioDevice {
    /// common config of one device
    pub common_cfg: SafePtr<VirtioPciCommonCfg, IoMem>,
    pub device: VirtioDevice,
    pub msix: MSIX,
}

impl PCIVirtioDevice {
    /// create a new PCI Virtio Device, note that this function will stop with device status features ok
    pub fn new(dev: Arc<PciDevice>) -> Self {
        assert_eq!(dev.id.vendor_id, 0x1af4);
        let device_type = match dev.id.device_id {
            0x1000 | 0x1041 => VirtioDeviceType::Network,
            0x1001 | 0x1042 => VirtioDeviceType::Block,
            0x1002 | 0x1043 => VirtioDeviceType::TraditionalMemoryBalloon,
            0x1003 | 0x1044 => VirtioDeviceType::Console,
            0x1004 | 0x1045 => VirtioDeviceType::ScsiHost,
            0x1005 | 0x1046 => VirtioDeviceType::Entropy,
            // 0x1009 | 0x104a => VirtioDeviceType::,
            0x1011 | 0x1052 => VirtioDeviceType::Input,
            _ => {
                panic!("initialize PCIDevice failed, unrecognized Virtio Device Type")
            }
        };
        info!("PCI device:{:?}", device_type);
        let bars = dev.bars;
        let loc = dev.loc;
        let mut msix = MSIX::default();
        let mut virtio_cap_list = Vec::new();
        for cap in dev.capabilities.iter() {
            match &cap.data {
                jinux_pci::capability::CapabilityData::VNDR(_) => {
                    virtio_cap_list.push(cap);
                }
                jinux_pci::capability::CapabilityData::MSIX(cap_data) => {
                    msix = MSIX::new(&cap_data, bars, loc, cap.cap_ptr);
                }
                jinux_pci::capability::CapabilityData::Unknown(id) => {
                    panic!("unknown capability device:{}", id)
                }
                _ => {
                    panic!("PCI Virtio device should not have other type of capability")
                }
            }
        }
        // create device
        let virtio_info = VirtioInfo::new(device_type, bars, virtio_cap_list).unwrap();
        let mut msix_vector_list: Vec<u16> = (0..msix.table_size).collect();
        let config_msix_vector = msix_vector_list.pop().unwrap();
        let common_cfg = &virtio_info.common_cfg_frame_ptr;

        // Reset device
        field_ptr!(common_cfg, VirtioPciCommonCfg, device_status)
            .write(&0u8)
            .unwrap();

        field_ptr!(common_cfg, VirtioPciCommonCfg, config_msix_vector)
            .write(&config_msix_vector)
            .unwrap();
        field_ptr!(common_cfg, VirtioPciCommonCfg, device_status)
            .write(&(DeviceStatus::ACKNOWLEDGE | DeviceStatus::DRIVER).bits())
            .unwrap();
        // negotiate features
        // get the value of device features
        field_ptr!(common_cfg, VirtioPciCommonCfg, device_feature_select)
            .write(&0u32)
            .unwrap();
        let mut low: u32 = field_ptr!(common_cfg, VirtioPciCommonCfg, device_feature)
            .read()
            .unwrap();
        field_ptr!(common_cfg, VirtioPciCommonCfg, device_feature_select)
            .write(&1u32)
            .unwrap();
        let mut high: u32 = field_ptr!(common_cfg, VirtioPciCommonCfg, device_feature)
            .read()
            .unwrap();
        let mut feature = (high as u64) << 32;
        feature |= low as u64;
        // let the device to negotiate Features
        let driver_features = VirtioDevice::negotiate_features(feature, device_type);
        // write features back
        low = driver_features as u32;
        high = (driver_features >> 32) as u32;
        field_ptr!(common_cfg, VirtioPciCommonCfg, driver_feature_select)
            .write(&0u32)
            .unwrap();
        field_ptr!(common_cfg, VirtioPciCommonCfg, driver_feature)
            .write(&low)
            .unwrap();
        field_ptr!(common_cfg, VirtioPciCommonCfg, driver_feature_select)
            .write(&1u32)
            .unwrap();
        field_ptr!(common_cfg, VirtioPciCommonCfg, driver_feature)
            .write(&high)
            .unwrap();
        // change to features ok status
        field_ptr!(common_cfg, VirtioPciCommonCfg, device_status)
            .write(
                &(DeviceStatus::ACKNOWLEDGE | DeviceStatus::DRIVER | DeviceStatus::FEATURES_OK)
                    .bits(),
            )
            .unwrap();
        let device = VirtioDevice::new(&virtio_info, bars, msix_vector_list).unwrap();
        // change to driver ok status
        field_ptr!(common_cfg, VirtioPciCommonCfg, device_status)
            .write(
                &(DeviceStatus::ACKNOWLEDGE
                    | DeviceStatus::DRIVER
                    | DeviceStatus::FEATURES_OK
                    | DeviceStatus::DRIVER_OK)
                    .bits(),
            )
            .unwrap();
        Self {
            common_cfg: virtio_info.common_cfg_frame_ptr,
            device,
            msix,
        }
    }

    /// register all the interrupt functions, this function should call only once
    pub fn register_interrupt_functions<F, T>(
        &mut self,
        config_change_function: &'static F,
        other_function: &'static T,
    ) where
        F: Fn(&TrapFrame) + Send + Sync + 'static,
        T: Fn(&TrapFrame) + Send + Sync + 'static,
    {
        let config_msix_vector =
            field_ptr!(&self.common_cfg, VirtioPciCommonCfg, config_msix_vector)
                .read()
                .unwrap() as usize;
        for i in 0..self.msix.table_size as usize {
            let msix = self.msix.table.get_mut(i).unwrap();
            if !msix.irq_handle.is_empty() {
                panic!("function `register_queue_interrupt_functions` called more than one time");
            }
            if config_msix_vector == i {
                msix.irq_handle.on_active(config_change_function);
            } else {
                msix.irq_handle.on_active(other_function);
            }
        }
    }
}
