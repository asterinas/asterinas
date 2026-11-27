// SPDX-License-Identifier: MPL-2.0

//! Manages virtio entropy devices.
//!
//! This module owns the global registry of discovered [`EntropyDevice`] instances.
//! Virtio transport initialization creates devices in [`device`], then registers
//! them here under stable names such as `virtio_rng.0`.

use alloc::{collections::btree_map::BTreeMap, sync::Arc, vec::Vec};

use ostd::sync::SpinLock;

use crate::device::entropy::device::EntropyDevice;

pub mod device;
pub(crate) mod driver;

/// Returns the names of the registered entropy sources.
pub fn available_devices() -> Vec<Arc<str>> {
    ENTROPY_REGISTRY.lock().devices.keys().cloned().collect()
}

/// Returns the entropy source for the next read from `/dev/hwrng`.
pub fn current_device() -> Option<Arc<EntropyDevice>> {
    ENTROPY_REGISTRY.lock().current.clone()
}

/// Selects a registered entropy source by name.
/// An empty name selects the first source in name order.
/// An unknown name leaves the current selection in place.
pub fn select_device(name: &str) {
    let mut registry = ENTROPY_REGISTRY.lock();
    if name.is_empty() {
        registry.current = registry.devices.values().next().cloned();
    } else if let Some(device) = registry.devices.get(name) {
        registry.current = Some(device.clone());
    }
}

fn register_device(name: Arc<str>, device: Arc<EntropyDevice>) {
    let mut registry = ENTROPY_REGISTRY.lock();
    if registry.current.is_none() {
        registry.current = Some(device.clone());
    }
    registry.devices.insert(name, device);
}

static ENTROPY_REGISTRY: SpinLock<EntropyRegistry> = SpinLock::new(EntropyRegistry::new());

struct EntropyRegistry {
    devices: BTreeMap<Arc<str>, Arc<EntropyDevice>>,
    current: Option<Arc<EntropyDevice>>,
}

impl EntropyRegistry {
    const fn new() -> Self {
        Self {
            devices: BTreeMap::new(),
            current: None,
        }
    }
}
