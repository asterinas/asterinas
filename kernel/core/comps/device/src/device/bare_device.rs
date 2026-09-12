// SPDX-License-Identifier: MPL-2.0

//! Devices with neither bus nor class.

use alloc::{
    string::String,
    sync::{Arc, Weak},
    vec::Vec,
};

use super::{AnyDevice, DevNode, DeviceBase, DeviceInternals, Subsystem, impl_device_node};
use crate::{SysStr, attr::TyErasedAttr, uevent::UeventVars};

/// A device with neither bus nor class, such as a host bridge or a firmware
/// root.
///
/// It has no attributes beyond `uevent`, is listed in no index, and sends no
/// uevents; it exists to be the parent of other devices.
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
    pub fn new_root(name: impl Into<SysStr>) -> Arc<Self> {
        Self::new(name, None)
    }

    /// Creates a bare device under `parent`.
    ///
    /// It is not registered until [`add`](super::add) is called.
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

    fn subsystem_uevent(&self, _vars: &mut UeventVars) {}

    fn devnode_override(&self) -> Option<DevNode> {
        None
    }
}

impl_device_node!(BareDevice);
