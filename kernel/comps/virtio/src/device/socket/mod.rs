// SPDX-License-Identifier: MPL-2.0

// ! #![feature(linked_list_cursors)]
use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};

use ostd::sync::SpinLock;
use spin::Once;

use self::device::SocketDevice;
pub mod buffer;
pub mod config;
pub mod connect;
pub mod device;
pub mod error;
pub mod header;

pub static DEVICE_NAME: &str = "Virtio-Vsock";
pub trait VsockDeviceIrqHandler = Fn() + Send + Sync + 'static;

pub fn register_device(name: String, device: Arc<SpinLock<SocketDevice>>) {
    VSOCK_DEVICE_TABLE
        .get()
        .unwrap()
        .lock_irq_disabled()
        .insert(name, (Arc::new(SpinLock::new(Vec::new())), device));
}

pub fn get_device(str: &str) -> Option<Arc<SpinLock<SocketDevice>>> {
    let lock = VSOCK_DEVICE_TABLE.get().unwrap().lock_irq_disabled();
    let (_, device) = lock.get(str)?;
    Some(device.clone())
}

pub fn all_devices() -> Vec<(String, Arc<SpinLock<SocketDevice>>)> {
    let vsock_devs = VSOCK_DEVICE_TABLE.get().unwrap().lock_irq_disabled();
    vsock_devs
        .iter()
        .map(|(name, (_, device))| (name.clone(), device.clone()))
        .collect()
}

pub fn register_recv_callback(name: &str, callback: impl VsockDeviceIrqHandler) {
    let lock = VSOCK_DEVICE_TABLE.get().unwrap().lock_irq_disabled();
    let Some((callbacks, _)) = lock.get(name) else {
        return;
    };
    callbacks.lock_irq_disabled().push(Arc::new(callback));
}

pub fn handle_recv_irq(name: &str) {
    let lock = VSOCK_DEVICE_TABLE.get().unwrap().lock_irq_disabled();
    let Some((callbacks, _)) = lock.get(name) else {
        return;
    };
    let lock = callbacks.lock_irq_disabled();
    for callback in lock.iter() {
        callback.call(())
    }
}

pub fn init() {
    VSOCK_DEVICE_TABLE.call_once(|| SpinLock::new(BTreeMap::new()));
    buffer::init();
}

type VsockDeviceIrqHandlerListRef = Arc<SpinLock<Vec<Arc<dyn VsockDeviceIrqHandler>>>>;
type VsockDeviceRef = Arc<SpinLock<SocketDevice>>;

pub static VSOCK_DEVICE_TABLE: Once<
    SpinLock<BTreeMap<String, (VsockDeviceIrqHandlerListRef, VsockDeviceRef)>>,
> = Once::new();
