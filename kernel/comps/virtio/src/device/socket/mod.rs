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
        .disable_irq()
        .lock_with(|table| table.insert(name, (Arc::new(SpinLock::new(Vec::new())), device)));
}

pub fn get_device(str: &str) -> Option<Arc<SpinLock<SocketDevice>>> {
    VSOCK_DEVICE_TABLE
        .get()
        .unwrap()
        .disable_irq()
        .lock_with(|lock| {
            let (_, device) = lock.get(str)?;
            Some(device.clone())
        })
}

pub fn all_devices() -> Vec<(String, Arc<SpinLock<SocketDevice>>)> {
    VSOCK_DEVICE_TABLE
        .get()
        .unwrap()
        .disable_irq()
        .lock_with(|vsock_devs| {
            vsock_devs
                .iter()
                .map(|(name, (_, device))| (name.clone(), device.clone()))
                .collect()
        })
}

pub fn register_recv_callback(name: &str, callback: impl VsockDeviceIrqHandler) {
    VSOCK_DEVICE_TABLE
        .get()
        .unwrap()
        .disable_irq()
        .lock_with(|lock| {
            let Some((callbacks, _)) = lock.get(name) else {
                return;
            };
            callbacks
                .disable_irq()
                .lock_with(|callbacks| callbacks.push(Arc::new(callback)));
        });
}

pub fn handle_recv_irq(name: &str) {
    VSOCK_DEVICE_TABLE
        .get()
        .unwrap()
        .disable_irq()
        .lock_with(|lock| {
            let Some((callbacks, _)) = lock.get(name) else {
                return;
            };
            callbacks.disable_irq().lock_with(|lock| {
                for callback in lock.iter() {
                    callback.call(())
                }
            });
        });
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
