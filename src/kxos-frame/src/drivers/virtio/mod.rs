use core::mem::size_of;

use crate::mm;
use bitflags::bitflags;
use pci::{CSpaceAccessMethod, Location};

use self::block::VirtioBLKConfig;

use super::pci::*;

pub(crate) mod block;
pub(crate) mod queue;

pub(crate) const PCI_VIRTIO_CAP_COMMON_CFG: u8 = 1;
pub(crate) const PCI_VIRTIO_CAP_NOTIFY_CFG: u8 = 2;
pub(crate) const PCI_VIRTIO_CAP_ISR_CFG: u8 = 3;
pub(crate) const PCI_VIRTIO_CAP_DEVICE_CFG: u8 = 4;
pub(crate) const PCI_VIRTIO_CAP_PCI_CFG: u8 = 5;

bitflags! {
    /// The device status field.
    pub(crate) struct DeviceStatus: u8 {
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

#[derive(Debug)]
#[repr(C)]
pub(crate) struct VitrioPciCommonCfg {
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

#[derive(Debug)]
enum CFGType {
    COMMON(&'static mut VitrioPciCommonCfg),
    NOTIFY(u32),
    ISR,
    DEVICE(VirtioDeviceCFG),
    PCI,
}

#[derive(Debug)]
enum VirtioDeviceCFG {
    Network,
    Block(&'static mut VirtioBLKConfig),
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
struct PciVirtioCapability {
    pub cap_vndr: u8,
    pub cap_ptr: u16,
    pub cap_len: u8,
    pub cfg_type: u8,
    pub cfg: CFGType,
    pub bar: u8,
    pub offset: u32,
    pub length: u32,
}

impl VitrioPciCommonCfg {
    pub unsafe fn new(loc: Location, cap_ptr: u16) -> &'static mut Self {
        let ops = &PortOpsImpl;
        let am = CSpaceAccessMethod::IO;
        let bar = am.read8(ops, loc, cap_ptr + 4);
        let offset = am.read32(ops, loc, cap_ptr + 8);
        let bar_address = am.read32(ops, loc, PCI_BAR + bar as u16 * 4) & (!(0b1111));
        &mut *(mm::phys_to_virt(bar_address as usize + offset as usize) as *const usize
            as *mut Self)
    }
}

impl PciVirtioCapability {
    pub unsafe fn handle(loc: Location, cap_ptr: u16) -> Self {
        let ops = &PortOpsImpl;
        let am = CSpaceAccessMethod::IO;
        let cap_vndr = am.read8(ops, loc, cap_ptr);
        let cap_next = am.read8(ops, loc, cap_ptr + 1);
        let cap_len = am.read8(ops, loc, cap_ptr + 2);
        let cfg_type = am.read8(ops, loc, cap_ptr + 3);
        let cfg = match cfg_type {
            PCI_VIRTIO_CAP_COMMON_CFG => CFGType::COMMON(VitrioPciCommonCfg::new(loc, cap_ptr)),
            PCI_VIRTIO_CAP_NOTIFY_CFG => CFGType::NOTIFY(am.read32(ops, loc, cap_ptr + 16)),
            PCI_VIRTIO_CAP_ISR_CFG => CFGType::ISR,
            PCI_VIRTIO_CAP_DEVICE_CFG => {
                CFGType::DEVICE(VirtioDeviceCFG::Block(VirtioBLKConfig::new(loc, cap_ptr)))
            }
            PCI_VIRTIO_CAP_PCI_CFG => CFGType::PCI,
            _ => panic!("unsupport cfg, cfg_type:{}", cfg_type),
        };
        let cap = PciVirtioCapability {
            cap_vndr: cap_vndr,
            cap_ptr: cap_ptr,
            cap_len: cap_len,
            cfg_type: cfg_type,
            cfg: cfg,
            bar: am.read8(ops, loc, cap_ptr + 4),
            offset: am.read32(ops, loc, cap_ptr + 8),
            length: am.read32(ops, loc, cap_ptr + 12),
        };
        cap
    }
}

/// Convert a struct into a byte buffer.
unsafe trait AsBuf: Sized {
    fn as_buf(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self as *const _ as _, size_of::<Self>()) }
    }
    fn as_buf_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self as *mut _ as _, size_of::<Self>()) }
    }
}
