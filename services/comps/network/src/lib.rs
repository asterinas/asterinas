#![no_std]
#![forbid(unsafe_code)]
#![feature(trait_alias)]

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec::Vec;
use component::init_component;
use component::ComponentInitError;
use core::any::Any;
use jinux_frame::sync::SpinLock;
use jinux_virtio::device::network::device::EthernetAddr;
use jinux_virtio::VirtioDeviceType;
use spin::Once;

mod driver;
mod virtio;

pub use virtio::VirtioNet;

pub trait NetworkDevice: Send + Sync + Any {
    fn irq_number(&self) -> u8;
    fn name(&self) -> &'static str;
    fn mac_addr(&self) -> EthernetAddr;
}

pub trait NetDeviceIrqHandler = Fn(u8) + Send + Sync + 'static;
pub(crate) static NETWORK_IRQ_HANDLERS: Once<SpinLock<Vec<Arc<dyn NetDeviceIrqHandler>>>> =
    Once::new();

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    NETWORK_IRQ_HANDLERS.call_once(|| SpinLock::new(Vec::new()));
    Ok(())
}

pub fn probe_virtio_net() -> Result<VirtioNet, ComponentInitError> {
    let network_devices = {
        let virtio = jinux_virtio::VIRTIO_COMPONENT.get().unwrap();
        virtio.get_device(VirtioDeviceType::Network)
    };

    for device in network_devices {
        let virtio_net = VirtioNet::new(device);
        // FIXME: deal with multiple net devices
        return Ok(virtio_net);
    }

    Err(ComponentInitError::Unknown)
}

pub fn register_net_device_irq_handler(callback: impl NetDeviceIrqHandler) {
    NETWORK_IRQ_HANDLERS
        .get()
        .unwrap()
        .lock()
        .push(Arc::new(callback))
}
