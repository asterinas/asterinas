// SPDX-License-Identifier: MPL-2.0

//! Devices with neither bus nor class.

use alloc::{
    string::String,
    sync::{Arc, Weak},
    vec::Vec,
};

use super::{AnyDevice, DevNode, DeviceBase, DeviceInternals, impl_device_node};
use crate::{Subsystem, SysStr, attr::TyErasedAttr};

/// A device with neither bus nor class, such as a host bridge or a firmware root.
///
/// It has no attributes and is listed in no index; it exists to be the parent of other devices.
pub struct BareDevice {
    base: DeviceBase,
}

impl core::fmt::Debug for BareDevice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BareDevice")
            .field("base", &self.base)
            .finish()
    }
}

impl BareDevice {
    /// Creates a bare device at the top of `/sys/devices`.
    ///
    /// It is not registered until [`add`](super::add) is called.
    ///
    /// # Panics
    ///
    /// Panics if `name` is not a valid `SysTree` node name.
    pub fn new_root(name: impl Into<SysStr>) -> Arc<Self> {
        Self::new(name, None)
    }

    /// Same as [`Self::new_root`], except the device is created under `parent`.
    pub fn with_parent(name: impl Into<SysStr>, parent: Arc<dyn AnyDevice>) -> Arc<Self> {
        Self::new(name, Some(parent))
    }

    fn new(name: impl Into<SysStr>, parent: Option<Arc<dyn AnyDevice>>) -> Arc<Self> {
        Arc::new_cyclic(|weak: &Weak<BareDevice>| {
            let weak_self: Weak<dyn AnyDevice> = weak.clone();
            BareDevice {
                base: DeviceBase::new(name.into(), parent, None, weak_self),
            }
        })
    }
}

impl AnyDevice for BareDevice {
    fn base(&self) -> &DeviceBase {
        &self.base
    }

    fn subsystem(&self) -> Subsystem {
        Subsystem::bare()
    }

    fn driver_name(&self) -> Option<String> {
        None
    }
}

impl DeviceInternals for BareDevice {
    fn type_name(&self) -> Option<&'static str> {
        None
    }

    fn attr_groups(&self) -> Vec<TyErasedAttr> {
        Vec::new()
    }

    fn devnode_override(&self) -> Option<DevNode> {
        None
    }
}

impl_device_node!(BareDevice);
