// SPDX-License-Identifier: MPL-2.0

use alloc::boxed::Box;
use core::fmt::Debug;

use aster_util::safe_ptr::SafePtr;
use log::{info, warn};
use ostd::{
    bus::{
        pci::{capability::CapabilityData, cfg_space::Bar, common_device::PciCommonDevice},
        BusProbeError,
    },
    io_mem::IoMem,
    mm::{DmaCoherent, HasDaddr, PAGE_SIZE},
    trap::IrqCallbackFunction,
};

use crate::{
    queue::UsedElem,
    transport::{
        pci::msix::VirtioMsixManager, AvailRing, ConfigManager, Descriptor, UsedRing,
        VirtioTransport, VirtioTransportError,
    },
    DeviceStatus, VirtioDeviceType,
};

// When used through the legacy interface, the virtio common configuration structure looks as follows:
//
//                          virtio legacy ==> Mapped into PCI BAR0
//  +---------------------------------------------------------------------------------------+
//  |                       Device Features Bits[0:31] (Read-only)                          |
//  +---------------------------------------------------------------------------------------+
//  |                       Driver Features Bits[0:31] (Read & Write)                       |
//  +---------------------------------------------------------------------------------------+
//  |                              Virtqueue Address PFN (R/W)                              |
//  +-------------------------------------------+-------------------------------------------+
//  |             Queue Select (R/W)            |               Queue Size (R)              |
//  +---------------------+---------------------+-------------------------------------------+
//  |    ISR Status (R)   | Device Status (R/W) |             Queue Notify (R/W)            |
//  +---------------------+---------------------+-------------------------------------------+
//  |          MSIX Queue Vector (R/W)          |          MSIX Config Vector (R/W)         |
//  +-------------------------------------------+-------------------------------------------+
//  |                          Device Specific Configurations (R/W)                         |
//  +---------------------------------------------------------------------------------------+
//
// When MSI-X capability is enabled, device-specific configuration starts at byte offset 24 in
// virtio common configuration structure. When MSI-X capability is not enabled, device-specific
// configuration starts at byte offset 20 in virtio header.
const DEVICE_FEATURES_OFFSET: usize = 0x00;
const DRIVER_FEATURES_OFFSET: usize = 0x04;
const QUEUE_ADDR_PFN_OFFSET: usize = 0x08;
const QUEUE_SIZE_OFFSET: usize = 0x0c;
const QUEUE_SELECT_OFFSET: usize = 0x0e;
const QUEUE_NOTIFY_OFFSET: usize = 0x10;
const DEVICE_STATUS_OFFSET: usize = 0x12;
const ISR_STATUS_OFFSET: usize = 0x13;
// If MSI-X is enabled for the device, there are two additional fields.
const CONFIG_MSIX_VECTOR_OFFSET: usize = 0x14;
const QUEUE_MSIX_VECTOR_OFFSET: usize = 0x16;

const DEVICE_CONFIG_OFFSET: usize = 0x14;
const DEVICE_CONFIG_OFFSET_WITH_MSIX: usize = 0x18;

pub struct VirtioPciLegacyTransport {
    device_type: VirtioDeviceType,
    common_device: PciCommonDevice,
    config_bar: Bar,
    num_queues: u16,
    msix_manager: VirtioMsixManager,
}

impl VirtioPciLegacyTransport {
    pub const QUEUE_ALIGN_SIZE: usize = 4096;

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
            _ => {
                warn!(
                    "Unrecognized virtio-pci device id:{:x?}",
                    common_device.device_id().device_id
                );
                return Err((BusProbeError::ConfigurationSpaceError, common_device));
            }
        };
        info!("[Virtio]: Found device:{:?}", device_type);

        let config_bar = common_device.bar_manager().bar(0).unwrap();

        let mut num_queues = 0u16;
        while num_queues < u16::MAX {
            config_bar
                .write_once(QUEUE_SELECT_OFFSET, num_queues)
                .unwrap();
            let queue_size = config_bar.read_once::<u16>(QUEUE_SIZE_OFFSET).unwrap();
            if queue_size == 0 {
                break;
            }
            num_queues += 1;
        }

        // TODO: Support interrupt without MSI-X
        let mut msix = None;
        for cap in common_device.capabilities().iter() {
            match cap.capability_data() {
                CapabilityData::Msix(data) => {
                    msix = Some(data.clone());
                }
                _ => continue,
            }
        }
        let Some(msix) = msix else {
            return Err((BusProbeError::ConfigurationSpaceError, common_device));
        };
        let msix_manager = VirtioMsixManager::new(msix);

        Ok(Self {
            device_type,
            common_device,
            config_bar,
            num_queues,
            msix_manager,
        })
    }

    /// Calculate the aligned virtqueue size.
    ///
    /// According to the VirtIO spec v0.9.5:
    ///
    /// Each virtqueue occupies two or more physically-contiguous pages (defined, for
    /// the purposes of this specification, as 4096 bytes), and consists of three parts:
    /// +------------------+------------------------------------------------+-----------+
    /// | Descriptor Table | Available Ring (padding to next 4096 boundary) | Used Ring |
    /// +------------------+------------------------------------------------+-----------+
    ///
    /// More details can be found at <http://ozlabs.org/~rusty/virtio-spec/virtio-0.9.5.pdf>.
    pub(crate) fn calc_virtqueue_size_aligned(queue_size: usize) -> usize {
        let align_mask = Self::QUEUE_ALIGN_SIZE - 1;

        ((size_of::<Descriptor>() * queue_size + size_of::<u16>() * (3 + queue_size) + align_mask)
            & !align_mask)
            + ((size_of::<u16>() * 3 + size_of::<UsedElem>() * queue_size + align_mask)
                & !align_mask)
    }
}

impl VirtioTransport for VirtioPciLegacyTransport {
    fn device_type(&self) -> VirtioDeviceType {
        self.device_type
    }

    fn set_queue(
        &mut self,
        idx: u16,
        _queue_size: u16,
        descriptor_ptr: &SafePtr<Descriptor, DmaCoherent>,
        _avail_ring_ptr: &SafePtr<AvailRing, DmaCoherent>,
        _used_ring_ptr: &SafePtr<UsedRing, DmaCoherent>,
    ) -> Result<(), VirtioTransportError> {
        // When using the legacy interface, there was no mechanism to negotiate
        // the queue size! The transitional driver MUST retrieve the `Queue Size`
        // field from the device and MUST allocate the total number of bytes
        // (including descriptor, avail_ring and used_ring) for the virtqueue
        // according to the specific formula, see `calc_virtqueue_size_aligned`.
        let queue_addr = descriptor_ptr.daddr();
        let page_frame_number = (queue_addr / PAGE_SIZE) as u32;

        self.config_bar
            .write_once(QUEUE_SELECT_OFFSET, idx)
            .unwrap();
        debug_assert_eq!(
            self.config_bar
                .read_once::<u16>(QUEUE_SELECT_OFFSET)
                .unwrap(),
            idx
        );
        self.config_bar
            .write_once(QUEUE_ADDR_PFN_OFFSET, page_frame_number)
            .unwrap();
        Ok(())
    }

    fn notify_config(&self, _idx: usize) -> ConfigManager<u32> {
        let bar_space = Some((self.config_bar.clone(), QUEUE_NOTIFY_OFFSET));

        ConfigManager::new(None, bar_space)
    }

    fn num_queues(&self) -> u16 {
        self.num_queues
    }

    fn device_config_mem(&self) -> Option<IoMem> {
        None
    }

    fn device_config_bar(&self) -> Option<(Bar, usize)> {
        let bar = self.config_bar.clone();
        let base = if self.msix_manager.is_enabled() {
            DEVICE_CONFIG_OFFSET_WITH_MSIX
        } else {
            DEVICE_CONFIG_OFFSET
        };

        Some((bar, base))
    }

    fn read_device_features(&self) -> u64 {
        // Only Feature Bits 0 to 31 are accessible through the Legacy Interface.
        let features = self
            .config_bar
            .read_once::<u32>(DEVICE_FEATURES_OFFSET)
            .unwrap();
        features as u64
    }

    fn write_driver_features(&mut self, features: u64) -> Result<(), VirtioTransportError> {
        // When used through the Legacy Interface, Transitional Devices MUST assume that
        // Feature Bits 32 to 63 are not acknowledged by Driver.
        self.config_bar
            .write_once(DRIVER_FEATURES_OFFSET, features as u32)
            .unwrap();
        Ok(())
    }

    fn read_device_status(&self) -> DeviceStatus {
        let status = self
            .config_bar
            .read_once::<u8>(DEVICE_STATUS_OFFSET)
            .unwrap();
        DeviceStatus::from_bits_truncate(status)
    }

    fn write_device_status(&mut self, status: DeviceStatus) -> Result<(), VirtioTransportError> {
        let status = status.bits();
        self.config_bar
            .write_once(DEVICE_STATUS_OFFSET, status)
            .unwrap();
        Ok(())
    }

    // Set to driver ok status
    fn finish_init(&mut self) {
        self.write_device_status(
            DeviceStatus::ACKNOWLEDGE | DeviceStatus::DRIVER | DeviceStatus::DRIVER_OK,
        )
        .unwrap();
    }

    fn max_queue_size(&self, idx: u16) -> Result<u16, VirtioTransportError> {
        self.config_bar
            .write_once(QUEUE_SELECT_OFFSET, idx)
            .unwrap();
        debug_assert_eq!(
            self.config_bar
                .read_once::<u16>(QUEUE_SELECT_OFFSET)
                .unwrap(),
            idx
        );
        Ok(self.config_bar.read_once(QUEUE_SIZE_OFFSET).unwrap())
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
            if let Some(unused_irq) = self.msix_manager.pop_unused_irq() {
                unused_irq
            } else {
                warn!(
                    "{:?}: `single_interrupt` ignored: no more IRQ lines available",
                    self.device_type()
                );
                self.msix_manager.shared_irq_line()
            }
        } else {
            self.msix_manager.shared_irq_line()
        };
        irq.on_active(func);

        self.config_bar
            .write_once(QUEUE_SELECT_OFFSET, index)
            .unwrap();
        debug_assert_eq!(
            self.config_bar
                .read_once::<u16>(QUEUE_SELECT_OFFSET)
                .unwrap(),
            index
        );
        self.config_bar
            .write_once(QUEUE_MSIX_VECTOR_OFFSET, vector)
            .unwrap();
        Ok(())
    }

    fn register_cfg_callback(
        &mut self,
        func: Box<IrqCallbackFunction>,
    ) -> Result<(), VirtioTransportError> {
        let (vector, irq) = self.msix_manager.config_msix_irq();
        irq.on_active(func);

        self.config_bar
            .write_once(CONFIG_MSIX_VECTOR_OFFSET, vector)
            .unwrap();
        Ok(())
    }

    fn is_legacy_version(&self) -> bool {
        true
    }
}

impl Debug for VirtioPciLegacyTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PCIVirtioLegacyDevice")
            .field("common_device", &self.common_device)
            .finish()
    }
}
