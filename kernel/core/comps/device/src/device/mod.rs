// SPDX-License-Identifier: MPL-2.0

//! Devices, and the one registration sequence shared by all of them.
//!
//! A device is a directory under `/sys/devices` plus the facts the device
//! model needs: its parent in the hardware topology, the subsystem (bus or
//! class) that owns it, an optional device number, and an optional device
//! type. There are three concrete device structs, one per subsystem kind:
//!
//! - [`BusDevice<B>`] is enumerated on bus `B` and can be bound to a driver;
//! - [`ClassDevice<C>`] is created by a driver as the interface user space
//!   sees, and belongs to class `C`;
//! - [`BareDevice`] has neither and exists only to be a parent (a host bridge,
//!   a firmware root).
//!
//! All three share a [`DeviceBase`] and implement [`AnyDevice`], the erased
//! view the registration sequence works on. The concrete structs are what
//! drivers and classes see, so their payloads are typed; the erased view is
//! what the tree walker sees, so parents and children are `Arc<dyn AnyDevice>`.
//!
//! The submodules hold one concept each: `registration` the `add` and
//! `remove` sequences and the builder, and one module per device struct.

mod bare_device;
mod bus_device;
mod class_device;
mod registration;

use alloc::{
    collections::BTreeMap,
    string::String,
    sync::{Arc, Weak},
    vec::Vec,
};

use aster_systree::{SysBranchNode, SysObj};
use ostd::sync::{Mutex, RwMutex};
use spin::Once;

pub use self::{
    bare_device::BareDevice,
    bus_device::{BusDevice, BusDeviceBuilder},
    class_device::{ClassDevice, ClassDeviceBuilder},
    registration::{DeviceBuilder, add, remove},
};
use crate::{
    Error, Result, SysStr,
    attr::{Attr, AttrTable, TyErasedAttr},
    devnum::{DevNodeRequest, DevNum},
    node::{Dir, GlueDirs, SysTreeEdit},
    uevent::UeventVars,
};

/// The `/dev` node a device was registered with, and whether a `devnode`
/// callback chose its mode (Linux reports `DEVMODE` only if one did).
#[derive(Clone, Debug)]
pub(crate) struct DevNodeSpec {
    pub(crate) request: DevNodeRequest,
    pub(crate) has_mode: bool,
}

/// What a `devnode` hook may override for a device's `/dev` node.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DevNode {
    /// The node path relative to `/dev`; `None` keeps the device name.
    pub path: Option<SysStr>,
    /// The permission bits; `None` keeps the default.
    pub mode: Option<u16>,
}

/// A finer kind within one bus or class, such as `disk` versus `partition`
/// in the `block` class.
///
/// A type contributes attributes, uevent variables, and a `devnode` policy to
/// every device that carries it, and is reported to user space as `DEVTYPE`.
pub struct DeviceType<D: ?Sized + 'static> {
    /// The name reported as `DEVTYPE`.
    pub name: &'static str,
    /// Attributes every device of this type gets.
    pub attrs: &'static [Attr<D>],
    /// Adds type-specific uevent variables.
    pub uevent: Option<fn(&D, &mut UeventVars)>,
    /// Overrides the `/dev` node name or mode.
    pub devnode: Option<fn(&D) -> Option<DevNode>>,
    /// Whether a class device of this type gets the `device` symlink to its
    /// parent. Linux omits the link for one type only: block partitions.
    pub has_device_link: bool,
}

impl<D: ?Sized + 'static> DeviceType<D> {
    /// Creates a type with a name and nothing else.
    pub const fn named(name: &'static str) -> Self {
        Self {
            name,
            attrs: &[],
            uevent: None,
            devnode: None,
            has_device_link: true,
        }
    }
}

/// The per-type parts a bus or class device carries: its device type and its
/// own attributes.
pub(crate) struct DeclaredParts<D: ?Sized + 'static> {
    dev_type: Option<&'static DeviceType<D>>,
    own_attrs: &'static [Attr<D>],
}

impl<D: AnyDevice> DeclaredParts<D> {
    /// Returns the `DEVTYPE` name, if the device has a type.
    fn type_name(&self) -> Option<&'static str> {
        self.dev_type.map(|t| t.name)
    }

    /// Erases the subsystem's, the type's, and the device's own attributes,
    /// in that order.
    fn attr_groups(&self, subsystem_attrs: &[Attr<D>]) -> Vec<TyErasedAttr> {
        let mut attrs = TyErasedAttr::from_typed_slice(subsystem_attrs);
        if let Some(t) = self.dev_type {
            attrs.extend(TyErasedAttr::from_typed_slice(t.attrs));
        }
        attrs.extend(TyErasedAttr::from_typed_slice(self.own_attrs));
        attrs
    }

    /// Adds the type's uevent variables, if it has a hook.
    fn type_uevent(&self, dev: &D, vars: &mut UeventVars) {
        if let Some(uevent) = self.dev_type.and_then(|t| t.uevent) {
            uevent(dev, vars);
        }
    }

    /// Returns the type's `/dev` node override, if it has one.
    fn type_devnode(&self, dev: &D) -> Option<DevNode> {
        self.dev_type.and_then(|t| t.devnode).and_then(|f| f(dev))
    }

    /// Returns whether the type keeps the `device` symlink.
    fn has_device_link(&self) -> bool {
        self.dev_type.is_none_or(|t| t.has_device_link)
    }
}

/// The kind of subsystem a device belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubsystemKind {
    /// The device was enumerated on a bus.
    Bus,
    /// The device is the user-space face of something, in a class.
    Class,
    /// The device only exists to be a parent.
    Bare,
}

/// The subsystem that owns a device. A device has exactly one.
#[derive(Clone)]
pub struct Subsystem {
    kind: SubsystemKind,
    /// The bus or class handle; `None` for a bare device.
    ops: Option<Arc<dyn SubsystemOps>>,
}

impl core::fmt::Debug for Subsystem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Subsystem")
            .field("kind", &self.kind)
            .field("name", &self.name())
            .finish()
    }
}

impl Subsystem {
    pub(crate) fn bus(ops: Arc<dyn SubsystemOps>) -> Self {
        Self {
            kind: SubsystemKind::Bus,
            ops: Some(ops),
        }
    }

    pub(crate) fn class(ops: Arc<dyn SubsystemOps>) -> Self {
        Self {
            kind: SubsystemKind::Class,
            ops: Some(ops),
        }
    }

    pub(crate) const fn bare() -> Self {
        Self {
            kind: SubsystemKind::Bare,
            ops: None,
        }
    }

    /// Returns the kind of subsystem.
    pub fn kind(&self) -> SubsystemKind {
        self.kind
    }

    /// Returns the bus or class name, if any.
    pub fn name(&self) -> Option<&str> {
        self.ops.as_ref().map(|ops| ops.name())
    }

    /// Returns the handle behind the subsystem, if any.
    pub(crate) fn ops(&self) -> Option<&Arc<dyn SubsystemOps>> {
        self.ops.as_ref()
    }

    /// Returns the directory the device's `subsystem` link points to.
    pub(crate) fn dir(&self) -> Option<Arc<Dir>> {
        self.ops.as_ref().map(|ops| ops.dir())
    }

    /// Returns the directory that lists the device: `/sys/bus/<bus>/devices`
    /// or `/sys/class/<class>`.
    pub(crate) fn index_dir(&self) -> Option<Arc<Dir>> {
        self.ops.as_ref().map(|ops| ops.index_dir())
    }

    /// Returns whether a class device under a class-device parent still gets
    /// a glue directory.
    pub(crate) fn keeps_glue_dir(&self) -> bool {
        self.ops.as_ref().is_some_and(|ops| ops.keeps_glue_dir())
    }
}

/// What the registration sequence needs from a bus or class handle.
///
/// Implemented by [`BusHandle`](crate::BusHandle) and
/// [`ClassHandle`](crate::ClassHandle), and reachable only through
/// [`Subsystem`], whose field is private, so that the callbacks cannot be
/// invoked from outside the crate.
pub(crate) trait SubsystemOps: Send + Sync + 'static {
    /// Returns the bus or class name.
    fn name(&self) -> &'static str;

    /// Returns the `/sys/bus/<name>` or `/sys/class/<name>` directory.
    fn dir(&self) -> Arc<Dir>;

    /// Returns the directory that lists the subsystem's devices.
    fn index_dir(&self) -> Arc<Dir>;

    /// Returns whether a class device under a class-device parent still gets
    /// a glue directory (always false for a bus).
    fn keeps_glue_dir(&self) -> bool;

    /// Records a device that has just been registered and acts on it: a bus
    /// probes for a driver, a class notifies its interfaces.
    fn on_added(&self, dev: &Arc<dyn AnyDevice>);

    /// Forgets a device that is being removed: a bus unbinds it, a class
    /// notifies its interfaces.
    fn on_removed(&self, dev: &Arc<dyn AnyDevice>);
}

/// The life cycle of a device. Registration and removal each happen once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Built, not yet in the tree.
    Initialized,
    /// Registration in progress; the directory exists.
    Adding,
    /// Registered.
    Added,
    /// Removed; the object may still be referenced but is dead.
    Removed,
}

/// The symlinks a device's registration has created so far.
#[derive(Default)]
struct Links {
    subsystem: bool,
    device: bool,
    index: bool,
    dev_index: bool,
}

/// Where a device's directory was placed: a plain directory (a root, a glue
/// directory) or another device's directory.
enum TreeParent {
    Dir(Weak<Dir>),
    Device(Weak<dyn AnyDevice>),
}

impl TreeParent {
    /// Runs `f` with the crate-private editing view of the parent, if the
    /// parent is still alive.
    fn with_edit<R>(&self, f: impl FnOnce(&dyn SysTreeEdit) -> R) -> Option<R> {
        match self {
            TreeParent::Dir(dir) => dir.upgrade().map(|dir| f(dir.as_ref())),
            TreeParent::Device(dev) => dev.upgrade().map(|dev| f(dev.base())),
        }
    }
}

/// The part of a device that the registration sequence works on.
pub struct DeviceBase {
    id: aster_systree::SysNodeId,
    name: SysStr,
    /// The branch node this device's directory lives in, as `systree` sees
    /// it. Set when the device is attached.
    sys_parent: Once<Weak<dyn SysBranchNode>>,
    /// The same parent, as the registration sequence edits it.
    tree_parent: Once<TreeParent>,
    /// The entries of the device's directory: symlinks, glue directories, and
    /// child devices of any kind.
    children: RwMutex<BTreeMap<SysStr, Arc<dyn SysObj>>>,
    attrs: AttrTable,
    weak_self: Weak<dyn AnyDevice>,
    /// The device this one is reached through, if any.
    parent: Option<Arc<dyn AnyDevice>>,
    devnum: Option<DevNum>,
    state: Mutex<State>,
    /// Glue directories created under this device, one per class of child.
    glue_dirs: GlueDirs,
    /// Which of the symlinks that `add` may create exist, so that `teardown`
    /// removes only links this device made.
    links: Mutex<Links>,
    /// Devices whose parent is this one.
    child_devices: Mutex<Vec<Weak<dyn AnyDevice>>>,
    /// The `/dev` node created for this device, kept so that the uevent
    /// variables agree with it and so that it can be deleted.
    devnode: Mutex<Option<DevNodeSpec>>,
}

impl core::fmt::Debug for DeviceBase {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DeviceBase")
            .field("name", &self.name)
            .field("devnum", &self.devnum)
            .field("state", &*self.state.lock())
            .finish_non_exhaustive()
    }
}

impl DeviceBase {
    fn new(
        name: SysStr,
        parent: Option<Arc<dyn AnyDevice>>,
        devnum: Option<DevNum>,
        weak_self: Weak<dyn AnyDevice>,
    ) -> Self {
        Self {
            id: aster_systree::SysNodeId::new(),
            name,
            sys_parent: Once::new(),
            tree_parent: Once::new(),
            children: RwMutex::new(BTreeMap::new()),
            attrs: AttrTable::new(),
            weak_self,
            parent,
            devnum,
            state: Mutex::new(State::Initialized),
            glue_dirs: GlueDirs::new(),
            links: Mutex::new(Links::default()),
            child_devices: Mutex::new(Vec::new()),
            devnode: Mutex::new(None),
        }
    }

    /// Returns the device name, which is also its directory name.
    pub fn name(&self) -> &SysStr {
        &self.name
    }

    /// Returns the parent device, if any.
    pub fn parent(&self) -> Option<&Arc<dyn AnyDevice>> {
        self.parent.as_ref()
    }

    /// Returns the device number, if user space can open this device.
    pub fn devnum(&self) -> Option<DevNum> {
        self.devnum
    }

    /// Returns whether the device is currently registered.
    pub fn is_added(&self) -> bool {
        // A device counts as added as soon as its directory exists, so that a
        // driver's `probe`, which runs inside `add`, can register children.
        matches!(*self.state.lock(), State::Adding | State::Added)
    }

    /// Returns the `/dev` node the device was registered with, if
    /// registration has reached the step that creates it.
    fn devnode_spec(&self) -> Option<DevNodeSpec> {
        self.devnode.lock().clone()
    }

    /// Returns the registered child devices, skipping any that have been
    /// dropped.
    pub fn child_devices(&self) -> Vec<Arc<dyn AnyDevice>> {
        self.child_devices
            .lock()
            .iter()
            .filter_map(|w| w.upgrade())
            .collect()
    }
}

impl SysTreeEdit for DeviceBase {
    fn attach_child(&self, child: Arc<dyn SysObj>) -> Result<()> {
        let mut children = self.children.write();
        let name = child.name().clone();
        if children.contains_key(&name) {
            return Err(Error::NameConflict);
        }
        let weak: Weak<dyn SysBranchNode> = self.weak_self.clone();
        child.init_parent(weak);
        children.insert(name, child);
        Ok(())
    }

    fn detach_child(&self, name: &str) -> Result<Arc<dyn SysObj>> {
        self.children.write().remove(name).ok_or(Error::NotFound)
    }

    fn has_children(&self) -> bool {
        !self.children.read().is_empty()
    }

    /// Returns the device's path, computed as [`SysObj::path`] computes it.
    fn tree_path(&self) -> String {
        let Some(parent) = self.sys_parent.get().and_then(|w| w.upgrade()) else {
            return String::from(self.name.as_ref());
        };
        let mut path = String::from(parent.path().as_ref());
        if !parent.is_root() {
            path.push('/');
        }
        path.push_str(&self.name);
        path
    }
}

mod internals {
    use alloc::vec::Vec;

    use super::DevNode;
    use crate::{attr::TyErasedAttr, uevent::UeventVars};

    /// What the registration sequence asks a device for.
    ///
    /// These are plumbing, not a service to callers, so the trait is private
    /// to the crate; it is also what seals [`super::AnyDevice`].
    pub trait DeviceInternals {
        /// Returns the device type name, reported as `DEVTYPE`.
        fn type_name(&self) -> Option<&'static str>;

        /// Returns the attributes contributed by the subsystem, the type, and
        /// the device, erased.
        fn attr_groups(&self) -> Vec<TyErasedAttr>;

        /// Adds the subsystem's and the type's uevent variables.
        fn subsystem_uevent(&self, vars: &mut UeventVars);

        /// Returns the `/dev` node overrides from the type and the subsystem.
        fn devnode_override(&self) -> Option<DevNode>;

        /// Returns whether the device gets a `device` symlink to its parent
        /// (class devices only; a type may opt out, as Linux's partitions do).
        fn wants_device_link(&self) -> bool {
            true
        }
    }
}

pub(crate) use internals::DeviceInternals;

/// The erased view of any device.
///
/// This is what the registration sequence, the sysfs tree, and parents see.
/// Drivers and classes never need it: they receive the concrete
/// [`BusDevice`] or [`ClassDevice`].
///
/// The trait is sealed twice over, through [`Container`](crate::Container)
/// and through the crate-private `DeviceInternals`: only the three device
/// structs of this crate implement it.
pub trait AnyDevice: crate::Container + DeviceInternals {
    /// Returns the shared base.
    fn base(&self) -> &DeviceBase;

    /// Returns the subsystem that owns the device.
    fn subsystem(&self) -> Subsystem;

    /// Returns the name of the bound driver, for bus devices.
    fn driver_name(&self) -> Option<String>;

    /// Returns a strong reference to this device as a trait object.
    fn to_arc(&self) -> Arc<dyn AnyDevice> {
        self.base()
            .weak_self
            .upgrade()
            .expect("a device is only reachable through an `Arc`")
    }

    /// Returns whether this is a class device.
    fn is_class_device(&self) -> bool {
        self.subsystem().kind() == SubsystemKind::Class
    }
}

/// Implements the `SysTree` traits and [`Container`](crate::Container) for a
/// device struct by delegating to its [`DeviceBase`].
///
/// `SysBranchNode::remove_child` keeps its refusing default: the tree under a
/// device is edited only by the registration sequence, through `SysTreeEdit`.
macro_rules! impl_device_node {
    ($ty:ident $(< $p:ident : $bound:path >)?) => {
        impl$(<$p: $bound>)? ::aster_systree::SysObj for $ty$(<$p>)? {
            fn as_any(&self) -> &dyn core::any::Any {
                self
            }

            fn cast_to_node(&self) -> Option<::alloc::sync::Arc<dyn ::aster_systree::SysNode>> {
                self.base().weak_self.upgrade().map(|d| d as ::alloc::sync::Arc<dyn ::aster_systree::SysNode>)
            }

            fn cast_to_branch(&self) -> Option<::alloc::sync::Arc<dyn ::aster_systree::SysBranchNode>> {
                self.base().weak_self.upgrade().map(|d| d as ::alloc::sync::Arc<dyn ::aster_systree::SysBranchNode>)
            }

            fn id(&self) -> &::aster_systree::SysNodeId {
                &self.base().id
            }

            fn type_(&self) -> ::aster_systree::SysNodeType {
                ::aster_systree::SysNodeType::Branch
            }

            fn name(&self) -> &$crate::SysStr {
                &self.base().name
            }

            fn init_parent(&self, parent: ::alloc::sync::Weak<dyn ::aster_systree::SysBranchNode>) {
                self.base().sys_parent.call_once(|| parent);
            }

            fn parent(&self) -> Option<::alloc::sync::Arc<dyn ::aster_systree::SysBranchNode>> {
                self.base().sys_parent.get().and_then(|w| w.upgrade())
            }
        }

        impl$(<$p: $bound>)? ::aster_systree::SysNode for $ty$(<$p>)? {
            fn node_attrs(&self) -> ::alloc::sync::Arc<::aster_systree::SysAttrSet> {
                self.base().attrs.set()
            }

            fn is_attr_absent(&self, _name: &str) -> bool {
                false
            }

            fn read_attr(&self, name: &str, writer: &mut ::ostd::mm::VmWriter) -> aster_systree::Result<usize> {
                self.read_attr_at(name, 0, writer)
            }

            fn write_attr(&self, name: &str, reader: &mut ::ostd::mm::VmReader) -> aster_systree::Result<usize> {
                if !self.base().is_added() {
                    return Err(aster_systree::Error::IsDead);
                }
                self.base().attrs.store(self, name, reader)
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
                self.base().attrs.show(self, name, offset, writer)
            }

            fn write_attr_at(
                &self,
                name: &str,
                _offset: usize,
                reader: &mut ::ostd::mm::VmReader,
            ) -> aster_systree::Result<usize> {
                self.write_attr(name, reader)
            }

            fn perms(&self) -> ::aster_systree::SysPerms {
                ::aster_systree::SysPerms::DEFAULT_RW_PERMS
            }
        }

        impl$(<$p: $bound>)? ::aster_systree::SysBranchNode for $ty$(<$p>)? {
            fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&::alloc::sync::Arc<dyn ::aster_systree::SysObj>>)) {
                let children = self.base().children.read();
                f(children.get(name))
            }

            fn visit_children_with(
                &self,
                min_id: u64,
                f: &mut dyn for<'a> FnMut(&'a ::alloc::sync::Arc<dyn ::aster_systree::SysObj>) -> Option<()>,
            ) {
                let children = self.base().children.read();
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
                self.base().children.read().get(name).cloned()
            }
        }

        impl$(<$p: $bound>)? $crate::node::Sealed for $ty$(<$p>)? {}

        impl$(<$p: $bound>)? $crate::Container for $ty$(<$p>)? {}
    };
}

use impl_device_node;
