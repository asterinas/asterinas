use jinux_frame::offset_of;
use jinux_frame::sync::SpinLock;
use jinux_frame::trap::TrapFrame;
use jinux_pci::msix::MSIX;
use jinux_util::frame_ptr::InFramePtr;
use jinux_virtio::device::network::device::{self, EthernetAddr};
use jinux_virtio::PCIVirtioDevice;
use jinux_virtio::VirtioPciCommonCfg;
use log::debug;

use crate::{NetworkDevice, NETWORK_IRQ_HANDLERS};

pub struct VirtioNet {
    /// Network Device
    device: device::NetworkDevice,
    /// Own common cfg to avoid other devices access this frame
    _common_cfg: InFramePtr<VirtioPciCommonCfg>,
    _msix: SpinLock<MSIX>,
    irq_number: u8,
}

impl NetworkDevice for VirtioNet {
    fn irq_number(&self) -> u8 {
        self.irq_number
    }

    fn name(&self) -> &'static str {
        "virtio net"
    }

    fn mac_addr(&self) -> EthernetAddr {
        self.device.mac_addr()
    }
}

impl VirtioNet {
    pub(crate) fn new(virtio_device: PCIVirtioDevice) -> Self {
        let device = if let jinux_virtio::device::VirtioDevice::Network(network_device) =
            virtio_device.device
        {
            network_device
        } else {
            panic!("Invalid device type")
        };

        let common_cfg = virtio_device.common_cfg;
        let mut msix = virtio_device.msix;
        let config_msix_vector =
            common_cfg.read_at(offset_of!(VirtioPciCommonCfg, config_msix_vector)) as usize;

        let mut network_irq_num = 0;
        for i in 0..msix.table_size as usize {
            let msix_entry = msix.table.get_mut(i).unwrap();
            if !msix_entry.irq_handle.is_empty() {
                panic!("msix already have irq functions");
            }
            if config_msix_vector == i {
                debug!(
                    "network config space change irq number = {}",
                    msix_entry.irq_handle.num()
                );
                msix_entry.irq_handle.on_active(config_space_change);
            } else {
                network_irq_num = msix_entry.irq_handle.num();
                msix_entry.irq_handle.on_active(handle_network_event);
            }
        }
        debug_assert!(network_irq_num != 0);
        debug!("Network device irq num = {}", network_irq_num);
        let device = VirtioNet {
            device,
            _common_cfg: common_cfg,
            irq_number: network_irq_num,
            _msix: SpinLock::new(msix),
        };
        device
    }

    pub(crate) fn can_receive(&self) -> bool {
        self.device.can_receive()
    }

    pub(crate) fn can_send(&self) -> bool {
        self.device.can_send()
    }

    pub(crate) fn device_mut(&mut self) -> &mut device::NetworkDevice {
        &mut self.device
    }
}

/// Interrupt handler if network device config space changes
fn config_space_change(_: &TrapFrame) {
    debug!("network device config space change");
}

/// Interrupt handler if network device receives some packet
fn handle_network_event(trap_frame: &TrapFrame) {
    let irq_num = trap_frame.trap_num as u8;
    for callback in NETWORK_IRQ_HANDLERS.get().unwrap().lock().iter() {
        callback(irq_num);
    }
}
