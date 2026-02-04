// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::btree_map::BTreeMap, sync::Arc, vec::Vec};

use ostd::sync::SpinLock;
use spin::Once;

use crate::device::entropy::device::EntropyDevice;

pub mod device;

pub static ENTROPY_DEVICE_TABLE: Once<SpinLock<BTreeMap<usize, Arc<EntropyDevice>>>> = Once::new();

pub fn register_device(id: usize, device: Arc<EntropyDevice>) {
    ENTROPY_DEVICE_TABLE
        .get()
        .unwrap()
        .disable_irq()
        .lock()
        .insert(id, device);
}

pub fn get_device(id: usize) -> Option<Arc<EntropyDevice>> {
    let lock = ENTROPY_DEVICE_TABLE.get().unwrap().disable_irq().lock();
    let device = lock.get(&id)?;
    Some(device.clone())
}

pub fn all_devices() -> Vec<Arc<EntropyDevice>> {
    let entropy_devs = ENTROPY_DEVICE_TABLE.get().unwrap().disable_irq().lock();
    entropy_devs.values().cloned().collect()
}

pub fn init() {
    ENTROPY_DEVICE_TABLE.call_once(|| SpinLock::new(BTreeMap::new()));
}
