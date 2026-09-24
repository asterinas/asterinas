// SPDX-License-Identifier: MPL-2.0

//! Devices in a class.

use alloc::{
    string::String,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::ops::Deref;

use super::{
    AnyDevice, DeclaredParts, DevNode, DeviceBase, DeviceBuilder, DeviceInternals, impl_device_node,
};
use crate::{
    Subsystem, SysStr,
    attr::TyErasedAttr,
    class::{Class, ClassHandle},
};

/// A device in class `C`: the interface user space sees.
///
/// Dereferences to the class-specific payload `C::Device`.
pub struct ClassDevice<C: Class> {
    base: DeviceBase,
    class: Arc<ClassHandle<C>>,
    payload: C::Device,
    declared: DeclaredParts<Self>,
    weak: Weak<Self>,
}

/// Builds a [`ClassDevice`].
pub type ClassDeviceBuilder<C> =
    DeviceBuilder<Arc<ClassHandle<C>>, <C as Class>::Device, ClassDevice<C>>;

impl<C: Class> core::fmt::Debug for ClassDevice<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ClassDevice")
            .field("class", &C::NAME)
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl<C: Class> Deref for ClassDevice<C> {
    type Target = C::Device;

    fn deref(&self) -> &C::Device {
        &self.payload
    }
}

impl<C: Class> ClassDevice<C> {
    /// Starts building a device in `class`.
    pub fn builder(
        class: &Arc<ClassHandle<C>>,
        name: impl Into<SysStr>,
        payload: C::Device,
    ) -> ClassDeviceBuilder<C> {
        DeviceBuilder::new(class.clone(), name.into(), payload)
    }

    /// Returns the class this device is in.
    pub fn class(&self) -> &Arc<ClassHandle<C>> {
        &self.class
    }

    /// Returns the class-specific payload.
    pub fn payload(&self) -> &C::Device {
        &self.payload
    }

    /// Returns a strong reference to this device.
    pub(crate) fn this(&self) -> Arc<Self> {
        self.weak
            .upgrade()
            .expect("a device is only reachable through an `Arc`")
    }
}

impl<C: Class> ClassDeviceBuilder<C> {
    /// Builds the device.
    /// It is not registered until [`add`](super::add) is called.
    ///
    /// # Panics
    ///
    /// Panics if the device name is not a valid `SysTree` node name.
    pub fn build(self) -> Arc<ClassDevice<C>> {
        let declared = self.declared_parts();
        Arc::new_cyclic(|weak: &Weak<ClassDevice<C>>| {
            let weak_self: Weak<dyn AnyDevice> = weak.clone();
            ClassDevice {
                base: DeviceBase::new(self.name, self.parent, self.devnum, weak_self),
                class: self.handle,
                payload: self.payload,
                declared,
                weak: weak.clone(),
            }
        })
    }
}

impl<C: Class> AnyDevice for ClassDevice<C> {
    fn base(&self) -> &DeviceBase {
        &self.base
    }

    fn subsystem(&self) -> Subsystem {
        Subsystem::class(self.class.clone())
    }

    fn driver_name(&self) -> Option<String> {
        None
    }
}

impl<C: Class> DeviceInternals for ClassDevice<C> {
    fn type_name(&self) -> Option<&'static str> {
        self.declared.type_name()
    }

    fn attr_groups(&self) -> Vec<TyErasedAttr> {
        self.declared.attr_groups(self.class.class().dev_attrs())
    }

    fn devnode_override(&self) -> Option<DevNode> {
        // As in Linux's `device_get_devnode`: the type is asked first, and the
        // class only if the type named no node. A type that sets just a mode
        // still lets the class name the node, and the class's mode wins.
        let from_type = self.declared.type_devnode(self);
        if from_type.as_ref().is_some_and(|node| node.path.is_some()) {
            return from_type;
        }
        let from_class = self.class.class().devnode(self);
        match (from_type, from_class) {
            (None, class) => class,
            (Some(t), None) => Some(t),
            (Some(t), Some(c)) => Some(DevNode {
                path: c.path,
                mode: c.mode.or(t.mode),
            }),
        }
    }

    fn wants_device_link(&self) -> bool {
        self.declared.has_device_link()
    }
}

impl_device_node!(ClassDevice<C: Class>);
