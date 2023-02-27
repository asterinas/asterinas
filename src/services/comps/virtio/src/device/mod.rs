use crate::{device::block::device::BLKDevice, Feature, VirtioDeviceType, VitrioPciCommonCfg};
use alloc::vec::Vec;
use jinux_pci::{
    capability::{vendor::virtio::CapabilityVirtioData, Capability},
    util::BAR,
};
use jinux_util::frame_ptr::InFramePtr;

use self::input::device::InputDevice;

pub mod block;
pub mod input;

pub(crate) const PCI_VIRTIO_CAP_COMMON_CFG: u8 = 1;
pub(crate) const PCI_VIRTIO_CAP_NOTIFY_CFG: u8 = 2;
pub(crate) const PCI_VIRTIO_CAP_ISR_CFG: u8 = 3;
pub(crate) const PCI_VIRTIO_CAP_DEVICE_CFG: u8 = 4;
pub(crate) const PCI_VIRTIO_CAP_PCI_CFG: u8 = 5;

#[derive(Debug)]
pub enum VirtioDevice {
    Network,
    Block(BLKDevice),
    Console,
    Entropy,
    TraditionalMemoryBalloon,
    ScsiHost,
    GPU,
    Input(InputDevice),
    Crypto,
    Socket,
    Unknown,
}

#[derive(Debug)]
pub enum VirtioDeviceError {
    /// queues amount do not match the requirement
    /// first element is actual value, second element is expect value
    QueuesAmountDoNotMatch(u16, u16),
    /// unknown error of queue
    QueueUnknownError,
    /// The input virtio capability list contains invalid element
    CapabilityListError,
}

pub struct VirtioInfo {
    pub device_type: VirtioDeviceType,
    pub notify_base_address: u64,
    pub notify_off_multiplier: u32,
    pub common_cfg_frame_ptr: InFramePtr<VitrioPciCommonCfg>,
    pub device_cap_cfg: CapabilityVirtioData,
}

impl VirtioInfo {
    pub(crate) fn new(
        device_type: VirtioDeviceType,
        bars: [Option<BAR>; 6],
        virtio_cap_list: Vec<&Capability>,
    ) -> Result<Self, VirtioDeviceError> {
        let mut notify_base_address = 0;
        let mut notify_off_multiplier = 0;
        let mut common_cfg_frame_ptr_some = None;
        let mut device_cap_cfg = None;
        for cap in virtio_cap_list.iter() {
            match cap.data {
                jinux_pci::capability::CapabilityData::VNDR(vndr_data) => match vndr_data {
                    jinux_pci::capability::vendor::CapabilityVNDRData::VIRTIO(cap_data) => {
                        match cap_data.cfg_type {
                            PCI_VIRTIO_CAP_COMMON_CFG => {
                                common_cfg_frame_ptr_some =
                                    Some(VitrioPciCommonCfg::new(&cap_data, bars));
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
                                device_cap_cfg = Some(cap_data);
                            }
                            PCI_VIRTIO_CAP_PCI_CFG => {}
                            _ => panic!("unsupport cfg, cfg_type:{}", cap_data.cfg_type),
                        };
                    }
                },
                _ => {
                    return Err(VirtioDeviceError::CapabilityListError);
                }
            }
        }
        Ok(Self {
            notify_base_address,
            notify_off_multiplier,
            common_cfg_frame_ptr: common_cfg_frame_ptr_some
                .ok_or(VirtioDeviceError::CapabilityListError)?,
            device_cap_cfg: device_cap_cfg.ok_or(VirtioDeviceError::CapabilityListError)?,
            device_type,
        })
    }
}

impl VirtioDevice {
    /// call this function after features_ok
    pub(crate) fn new(
        virtio_info: &VirtioInfo,
        bars: [Option<BAR>; 6],
        msix_vector_left: Vec<u16>,
    ) -> Result<Self, VirtioDeviceError> {
        let device = match virtio_info.device_type {
            VirtioDeviceType::Block => VirtioDevice::Block(BLKDevice::new(
                &virtio_info.device_cap_cfg,
                bars,
                &virtio_info.common_cfg_frame_ptr,
                virtio_info.notify_base_address as usize,
                virtio_info.notify_off_multiplier,
                msix_vector_left,
            )?),
            VirtioDeviceType::Input => VirtioDevice::Input(InputDevice::new(
                &virtio_info.device_cap_cfg,
                bars,
                &virtio_info.common_cfg_frame_ptr,
                virtio_info.notify_base_address as usize,
                virtio_info.notify_off_multiplier,
                msix_vector_left,
            )?),
            _ => {
                panic!("initialize PCIDevice failed, unsupport Virtio Device Type")
            }
        };
        Ok(device)
    }

    pub(crate) fn negotiate_features(features: u64, device_type: VirtioDeviceType) -> u64 {
        let device_specified_features = features & ((1 << 24) - 1);
        let device_support_features = match device_type {
            VirtioDeviceType::Network => todo!(),
            VirtioDeviceType::Block => BLKDevice::negotiate_features(device_specified_features),
            VirtioDeviceType::Console => todo!(),
            VirtioDeviceType::Entropy => todo!(),
            VirtioDeviceType::TraditionalMemoryBalloon => todo!(),
            VirtioDeviceType::ScsiHost => todo!(),
            VirtioDeviceType::GPU => todo!(),
            VirtioDeviceType::Input => InputDevice::negotiate_features(device_specified_features),
            VirtioDeviceType::Crypto => todo!(),
            VirtioDeviceType::Socket => todo!(),
            VirtioDeviceType::Unknown => todo!(),
        };
        let support_feature = Feature::from_bits_truncate(features);
        // support_feature.remove(Feature::RING_EVENT_IDX);
        features & (support_feature.bits | device_support_features)
    }
}
