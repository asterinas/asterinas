// SPDX-License-Identifier: MPL-2.0

//! Shared device types and attributes.
//!
//! Build a device through [`BusDevice::builder`] or [`ClassDevice::builder`],
//! then call [`add_device`] to register it.
//! Use [`BareDevice`] for a parent that belongs to neither a bus nor a class.
//! Remove child devices before calling [`remove_device`] on their parent.
//!
//! [`AnyDevice`] provides a common interface for working with devices
//! without knowing their concrete types.
//! Use [`aster_systree::SysBranchNode`] to traverse the sysfs hierarchy,
//! which may include glue directories between a device and its parent.
//!
//! Use [`Attr`] to define sysfs attributes and [`DeviceType`] to share attributes
//! and a device-node policy among devices of the same device type.
//! A [`DevNum`] identifies a character or block device and requests a `/dev` node during
//! registration.
//! [`DevNode`] lets a device type or class override the node's path or permissions.
//!
//! [`BusDevice::builder`]: crate::bus::BusDevice::builder
//! [`ClassDevice::builder`]: crate::class::ClassDevice::builder
//! [`add_device`]: crate::add_device
//! [`remove_device`]: crate::remove_device

pub(crate) mod attr;
mod bare_device;
mod devnum;
mod error;
mod internals;
pub(crate) mod node;
pub(crate) mod registration;
pub(crate) mod registry;
pub(crate) mod subsystem;

use alloc::{
    sync::{Arc, Weak},
    vec::Vec,
};

use aster_systree::SysBranchNode;

pub(crate) use self::internals::{DeviceBase, DeviceInternals};
use self::{
    attr::TyErasedAttr,
    node::{Dir, SysTreeEdit},
};
pub use self::{
    attr::{Attr, ShowFn, StoreFn},
    bare_device::BareDevice,
    devnum::{DEFAULT_DEVNODE_MODE, DevKind, DevNum},
    error::{Error, Result},
    registration::DeviceBuilder,
    subsystem::{Subsystem, SubsystemKind},
};

/// An owned string or a static reference to string.
pub type SysStr = aster_systree::SysStr;

/// An interface shared by all devices.
///
/// Use `dyn AnyDevice` to inspect devices without knowing their concrete bus or class type.
/// [`SysBranchNode`] provides access to its sysfs attributes and child nodes.
/// Creating or removing children through [`SysBranchNode`] is unsupported;
/// use [`add_device`] and [`remove_device`] to manage devices.
///
/// Implemented by [`BusDevice`], [`ClassDevice`], and [`BareDevice`].
/// This trait cannot be implemented outside this crate.
///
/// [`add_device`]: crate::add_device
/// [`remove_device`]: crate::remove_device
/// [`BusDevice`]: crate::bus::BusDevice
/// [`ClassDevice`]: crate::class::ClassDevice
pub trait AnyDevice: SysBranchNode + DeviceInternals {
    /// Returns the device number, if user space can open this device.
    fn devnum(&self) -> Option<DevNum> {
        self.base().devnum()
    }

    /// Returns the subsystem that owns the device.
    fn subsystem(&self) -> Subsystem;

    /// Returns a strong reference to this device as a trait object.
    fn to_arc(&self) -> Arc<dyn AnyDevice> {
        self.base()
            .fields()
            .weak_self()
            .upgrade()
            .expect("a device is only reachable through an `Arc`")
    }
}

/// A finer kind within one bus or class, such as `disk` versus `partition` in the `block` class.
///
/// A device type contributes attributes and a `devnode` policy to every device that carries it.
pub struct DeviceType<D: 'static> {
    /// The device type name.
    pub name: &'static str,
    /// Attributes contributed by this device type.
    pub attrs: &'static [Attr<D>],
    /// Overrides the `/dev` node name or mode.
    pub devnode: Option<fn(&D) -> Option<DevNode>>,
    /// Whether a class device with this device type gets the `device` symlink to its parent.
    pub has_device_link: bool,
}

impl<D: 'static> DeviceType<D> {
    /// Creates a device type with a name and nothing else.
    pub const fn named(name: &'static str) -> Self {
        Self {
            name,
            attrs: &[],
            devnode: None,
            has_device_link: true,
        }
    }
}

/// What a `devnode` hook may override for a device's `/dev` node.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DevNode {
    /// The node path relative to `/dev`; `None` keeps the device name.
    pub path: Option<SysStr>,
    /// The permission bits; `None` keeps the default.
    pub mode: Option<u16>,
}

/// The device type and device-specific attributes of a bus or class device.
pub(crate) struct DeclaredParts<D: 'static> {
    dev_type: Option<&'static DeviceType<D>>,
    own_attrs: &'static [Attr<D>],
}

impl<D: AnyDevice> DeclaredParts<D> {
    /// Returns the device type name, if the device has a device type.
    pub(crate) fn type_name(&self) -> Option<&'static str> {
        self.dev_type.map(|t| t.name)
    }

    /// Collects type-erased attributes from the subsystem, the device type, and the device, in that
    /// order.
    pub(crate) fn attr_groups(&self, subsystem_attrs: &[Attr<D>]) -> Vec<TyErasedAttr> {
        let mut attrs = TyErasedAttr::from_typed_slice(subsystem_attrs);
        if let Some(t) = self.dev_type {
            attrs.extend(TyErasedAttr::from_typed_slice(t.attrs));
        }
        attrs.extend(TyErasedAttr::from_typed_slice(self.own_attrs));
        attrs
    }

    /// Returns the device type's `/dev` node override, if it has one.
    pub(crate) fn type_devnode(&self, dev: &D) -> Option<DevNode> {
        self.dev_type.and_then(|t| t.devnode).and_then(|f| f(dev))
    }

    /// Returns whether the device type keeps the `device` symlink.
    pub(crate) fn has_device_link(&self) -> bool {
        self.dev_type.is_none_or(|t| t.has_device_link)
    }
}

/// The life cycle of a device.
/// Registration and removal each happen once.
///
/// Child devices are part of this state:
/// removing a device requires it to be added and have no children,
/// while adding a child requires checking the parent's state and recording the child together.
/// Keeping both under the state lock prevents a parent from being removed between these two steps.
#[derive(Debug)]
enum State {
    /// Built, not yet in the tree.
    Initialized,
    /// Preparing the directory and its resources; registration may still fail.
    /// Registration failure performs its own rollback.
    Preparing,
    /// Registration can no longer fail; subsystem callbacks may add children.
    /// Transitions to `Added` after the callbacks finish.
    Adding(ChildDevices),
    /// Registered.
    /// Only devices in this state can be removed.
    Added(ChildDevices),
    /// Removed; the object may still be referenced but is dead.
    Removed,
}

type ChildDevices = Vec<Weak<dyn AnyDevice>>;

/// The symlinks a device's registration has created so far.
#[derive(Default)]
struct Links {
    subsystem: bool,
    device: bool,
    index: bool,
    dev_index: bool,
}

/// Where a device's directory was placed:
/// a plain directory (a root, a glue directory) or another device's directory.
enum TreeParent {
    Dir(Weak<Dir>),
    Device(Weak<dyn AnyDevice>),
}

impl TreeParent {
    /// Runs `f` with the crate-private editing view of the parent, if the parent is still alive.
    fn with_edit<R>(&self, f: impl FnOnce(&dyn SysTreeEdit) -> R) -> Option<R> {
        match self {
            TreeParent::Dir(dir) => dir.upgrade().map(|dir| f(dir.as_ref())),
            TreeParent::Device(dev) => dev.upgrade().map(|dev| f(dev.base())),
        }
    }
}

/// Implements the `SysTree` traits for a device struct by delegating to its [`DeviceBase`].
///
/// `SysBranchNode::remove_child` keeps its refusing default:
/// the tree under a device is edited only by the registration sequence, through `SysTreeEdit`.
macro_rules! impl_device_node {
    ($ty:ident $(< $p:ident : $bound:path >)?) => {
        impl$(<$p: $bound>)? ::aster_systree::SysObj for $ty$(<$p>)? {
            fn as_any(&self) -> &dyn core::any::Any {
                self
            }

            fn cast_to_node(&self) -> Option<::alloc::sync::Arc<dyn ::aster_systree::SysNode>> {
                self.base()
                    .fields()
                    .weak_self()
                    .upgrade()
                    .map(|d| d as ::alloc::sync::Arc<dyn ::aster_systree::SysNode>)
            }

            fn cast_to_branch(
                &self
            ) -> Option<::alloc::sync::Arc<dyn ::aster_systree::SysBranchNode>> {
                self.base()
                    .fields()
                    .weak_self()
                    .upgrade()
                    .map(|d| d as ::alloc::sync::Arc<dyn ::aster_systree::SysBranchNode>)
            }

            fn id(&self) -> &::aster_systree::SysNodeId {
                self.base().fields().id()
            }

            fn type_(&self) -> ::aster_systree::SysNodeType {
                ::aster_systree::SysNodeType::Branch
            }

            fn name(&self) -> &$crate::common::SysStr {
                self.base().fields().name()
            }

            fn init_parent(&self, parent: ::alloc::sync::Weak<dyn ::aster_systree::SysBranchNode>) {
                self.base().fields().init_parent(parent);
            }

            fn parent(&self) -> Option<::alloc::sync::Arc<dyn ::aster_systree::SysBranchNode>> {
                self.base().fields().parent()
            }
        }

        impl$(<$p: $bound>)? ::aster_systree::SysNode for $ty$(<$p>)? {
            fn node_attrs(&self) -> ::alloc::sync::Arc<::aster_systree::SysAttrSet> {
                self.base().attrs().set()
            }

            fn is_attr_absent(&self, _name: &str) -> bool {
                false
            }

            fn read_attr(
                &self,
                name: &str,
                writer: &mut ::ostd::mm::VmWriter
            ) -> aster_systree::Result<usize> {
                self.read_attr_at(name, 0, writer)
            }

            fn write_attr_at(
                &self,
                name: &str,
                _offset: usize,
                reader: &mut ::ostd::mm::VmReader,
            ) -> aster_systree::Result<usize> {
                self.write_attr(name, reader)
            }

            fn write_attr(
                &self,
                name: &str,
                reader: &mut ::ostd::mm::VmReader
            ) -> aster_systree::Result<usize> {
                if !self.base().is_added() {
                    return Err(aster_systree::Error::IsDead);
                }
                self.base().attrs().store(self, name, reader)
            }

            fn read_attr_at(
                &self,
                name: &str,
                offset: usize,
                writer: &mut ::ostd::mm::VmWriter,
            ) -> aster_systree::Result<usize> {
                if !self.base().is_added() {
                    return Err(aster_systree::Error::IsDead);
                }
                self.base().attrs().show(self, name, offset, writer)
            }

            fn perms(&self) -> ::aster_systree::SysPerms {
                ::aster_systree::SysPerms::DEFAULT_RW_PERMS
            }
        }

        impl$(<$p: $bound>)? ::aster_systree::SysBranchNode for $ty$(<$p>)? {
            fn visit_child_with(
                &self,
                name: &str,
                f: &mut dyn FnMut(Option<&::alloc::sync::Arc<dyn ::aster_systree::SysObj>>)
            ) {
                let children = self.base().children().read();
                f(children.get(name))
            }

            fn visit_children_with(
                &self,
                min_id: u64,
                f: &mut dyn for<'a> FnMut(
                    &'a ::alloc::sync::Arc<dyn ::aster_systree::SysObj>
                ) -> Option<()>,
            ) {
                let children = self.base().children().read();
                for child in children.values() {
                    if child.id().as_u64() < min_id {
                        continue;
                    }
                    if f(child).is_none() {
                        break;
                    }
                }
            }

            fn child(&self, name: &str) -> Option<::alloc::sync::Arc<dyn ::aster_systree::SysObj>> {
                self.base().children().read().get(name).cloned()
            }
        }
    };
}

pub(crate) use impl_device_node;
