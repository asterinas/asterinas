// SPDX-License-Identifier: MPL-2.0

//! This mod is modified from virtio-drivers project.
use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};

use aster_frame::sync::SpinLock;
use component::ComponentInitError;
use spin::Once;

use self::device::SocketDevice;
pub mod buffer;
pub mod config;
pub mod connect;
pub mod device;
pub mod error;
pub mod header;
pub mod manager;

pub static DEVICE_NAME: &str = "Virtio-Vsock";
pub trait VsockDeviceIrqHandler = Fn() + Send + Sync + 'static;

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
    let device = lock.get(str)?;
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

pub fn component_init() -> Result<(), ComponentInitError> {
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
