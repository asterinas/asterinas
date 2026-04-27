// SPDX-License-Identifier: MPL-2.0

//! Manages virtio entropy devices.
//!
//! This module owns the global registry of discovered [`EntropyDevice`] instances.
//! Virtio transport initialization creates devices in [`device`], then registers
//! them here under stable names such as `virtio_rng.0`.

use alloc::{collections::btree_map::BTreeMap, string::String, sync::Arc};

use ostd::sync::SpinLock;
use spin::Once;

use crate::device::entropy::device::EntropyDevice;

pub mod device;

/// Registers an [`EntropyDevice`] under `name`.
fn register_device(name: String, device: Arc<EntropyDevice>) {
    let mut entropy_devs = ENTROPY_DEVICE_TABLE.get().unwrap().lock();

    entropy_devs.insert(name, device);
}

/// Returns the first registered [`EntropyDevice`].
pub fn first_device() -> Option<Arc<EntropyDevice>> {
    let entropy_devs = ENTROPY_DEVICE_TABLE.get().unwrap().lock();

    entropy_devs.values().next().cloned()
}

/// Initializes the entropy device registry.
pub(crate) fn init() {
    ENTROPY_DEVICE_TABLE.call_once(|| SpinLock::new(BTreeMap::new()));
}

static ENTROPY_DEVICE_TABLE: Once<SpinLock<BTreeMap<String, Arc<EntropyDevice>>>> = Once::new();
