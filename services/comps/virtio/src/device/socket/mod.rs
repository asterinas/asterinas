//! This mod is modified from virtio-drivers project.

use alloc::{sync::Arc, collections::BTreeMap, string::String, vec::Vec};
use component::{ComponentInitError, init_component};
use jinux_frame::sync::SpinLock;
use smoltcp::socket::dhcpv4::Socket;
use spin::Once;
use core::fmt::Debug;
use self::device::SocketDevice;
pub mod buffer;
pub mod config;
pub mod device;
pub mod header;
pub mod connect;
pub mod error;
pub mod manager;

pub static DEVICE_NAME: &str = "Virtio-Vsock";
pub type VsockDeviceIrqHandler = dyn Fn() + Send + Sync;


pub fn register_device(name: String, device: Arc<SpinLock<SocketDevice>>) {
    COMPONENT
        .get()
        .unwrap()
        .vsock_device_table
        .lock()
        .insert(name, device);
}

pub fn get_device(str: &str) -> Option<Arc<SpinLock<SocketDevice>>> {
    let lock = COMPONENT.get().unwrap().vsock_device_table.lock();
    let Some(device) = lock.get(str) else {
        return None;
    };
    Some(device.clone())
}

pub fn all_devices() -> Vec<(String, Arc<SpinLock<SocketDevice>>)> {
    let vsock_devs = COMPONENT.get().unwrap().vsock_device_table.lock();
    vsock_devs
        .iter()
        .map(|(name, device)| (name.clone(), device.clone()))
        .collect()
}

static COMPONENT: Once<Component> = Once::new();


pub fn component_init() -> Result<(), ComponentInitError>{
    let a = Component::init()?;
    COMPONENT.call_once(|| a);
    Ok(())
}


struct Component {
    vsock_device_table: SpinLock<BTreeMap<String, Arc<SpinLock<SocketDevice>>>>,
}

impl Component {
    pub fn init() -> Result<Self, ComponentInitError> {
        Ok(Self {
            vsock_device_table: SpinLock::new(BTreeMap::new()),
        })
    }
}