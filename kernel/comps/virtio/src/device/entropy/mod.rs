// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, collections::btree_map::BTreeMap, string::String, sync::Arc, vec::Vec};

use ostd::{
    arch::trap::TrapFrame,
    sync::{LocalIrqDisabled, SpinLock},
};
use spin::Once;

use crate::device::entropy::device::EntropyDevice;

pub mod device;

pub trait EntropyDeviceIrqHandler = Fn() + Send + Sync + 'static;

pub fn register_device(name: String, device: Arc<EntropyDevice>) {
    ENTROPY_DEVICE_TABLE
        .get()
        .unwrap()
        .lock()
        .insert(name, device);
}

pub fn get_device(str: &str) -> Option<Arc<EntropyDevice>> {
    let lock = ENTROPY_DEVICE_TABLE.get().unwrap().lock();
    lock.get(str).cloned()
}

pub fn all_devices() -> Vec<(String, Arc<EntropyDevice>)> {
    let entropy_devs = ENTROPY_DEVICE_TABLE.get().unwrap().lock();

    entropy_devs
        .iter()
        .map(|(name, dev)| (name.clone(), dev.clone()))
        .collect()
}

pub fn register_recv_callback(callback: impl EntropyDeviceIrqHandler) {
    ENTROPY_DEVICE_CALLBACK.call_once(|| Box::new(callback));
}

pub fn handle_recv_irq(_: &TrapFrame) {
    if let Some(callback) = ENTROPY_DEVICE_CALLBACK.get() {
        callback()
    }
}

pub fn init() {
    ENTROPY_DEVICE_TABLE.call_once(|| SpinLock::new(BTreeMap::new()));
}

pub static ENTROPY_DEVICE_CALLBACK: Once<Box<dyn EntropyDeviceIrqHandler>> = Once::new();

pub static ENTROPY_DEVICE_TABLE: Once<
    SpinLock<BTreeMap<String, Arc<EntropyDevice>>, LocalIrqDisabled>,
> = Once::new();
