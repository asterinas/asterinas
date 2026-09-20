// SPDX-License-Identifier: MPL-2.0

//! Internal device interfaces and shared state.

use alloc::{
    collections::BTreeMap,
    string::String,
    sync::{Arc, Weak},
    vec::Vec,
};

use aster_systree::{ObjFields, SysBranchNode, SysObj};
use ostd::sync::{Mutex, RwMutex};
use spin::Once;

use super::{AnyDevice, DevNode, Links, State, TreeParent};
use crate::{
    common::{
        Error, Result, SysStr,
        attr::{AttrTable, TyErasedAttr},
        devnum::DevNum,
        node::{GlueDirs, SysTreeEdit},
    },
    hooks::DevNodeRequest,
};

/// What the registration sequence asks a device for.
///
/// This trait seals [`AnyDevice`] and has no public import path.
pub trait DeviceInternals {
    /// Returns the shared device state.
    fn base(&self) -> &DeviceBase;

    /// Returns the device type name.
    fn type_name(&self) -> Option<&'static str>;

    /// Returns type-erased attributes contributed by the subsystem, the device type, and the device.
    fn attr_groups(&self) -> Vec<TyErasedAttr>;

    /// Returns the `/dev` node overrides from the device type and the subsystem.
    fn devnode_override(&self) -> Option<DevNode>;

    /// Returns whether the device gets a `device` symlink to its parent
    /// (class devices only; a device type may opt out).
    fn wants_device_link(&self) -> bool {
        true
    }
}

/// The properties and registration status shared by all devices.
pub struct DeviceBase {
    fields: ObjFields<dyn AnyDevice>,
    /// The parent in the sysfs tree, as the registration sequence edits it.
    tree_parent: Once<TreeParent>,
    /// The entries of the device's directory: symlinks, glue directories,
    /// and child devices of any kind.
    children: RwMutex<BTreeMap<SysStr, Arc<dyn SysObj>>>,
    attrs: AttrTable,
    /// The device this one is reached through, if any.
    parent: Option<Arc<dyn AnyDevice>>,
    devnum: Option<DevNum>,
    state: Mutex<State>,
    /// Glue directories created under this device, one per class of child.
    glue_dirs: GlueDirs,
    /// Which of the symlinks that `add` may create exist,
    /// so that `teardown` removes only links this device made.
    links: Mutex<Links>,
    /// The `/dev` node created for this device, kept so it can be deleted.
    devnode: Mutex<Option<DevNodeRequest>>,
}

impl DeviceBase {
    /// Returns the device name, which is also its directory name.
    pub(crate) fn name(&self) -> &SysStr {
        self.fields.name()
    }

    /// Returns the parent device, if any.
    pub(crate) fn parent(&self) -> Option<&Arc<dyn AnyDevice>> {
        self.parent.as_ref()
    }

    /// Returns whether the device is registered.
    pub(crate) fn is_added(&self) -> bool {
        matches!(*self.state.lock(), State::Adding(_) | State::Added(_))
    }

    /// Creates the shared state of a device.
    ///
    /// # Panics
    ///
    /// Panics if `name` is not a valid `SysTree` node name.
    pub(crate) fn new(
        name: SysStr,
        parent: Option<Arc<dyn AnyDevice>>,
        devnum: Option<DevNum>,
        weak_self: Weak<dyn AnyDevice>,
    ) -> Self {
        Self {
            fields: ObjFields::new(name, weak_self),
            tree_parent: Once::new(),
            children: RwMutex::new(BTreeMap::new()),
            attrs: AttrTable::new(),
            parent,
            devnum,
            state: Mutex::new(State::Initialized),
            glue_dirs: GlueDirs::new(),
            links: Mutex::new(Links::default()),
            devnode: Mutex::new(None),
        }
    }

    pub(crate) fn fields(&self) -> &ObjFields<dyn AnyDevice> {
        &self.fields
    }

    pub(crate) fn children(&self) -> &RwMutex<BTreeMap<SysStr, Arc<dyn SysObj>>> {
        &self.children
    }

    pub(crate) fn attrs(&self) -> &AttrTable {
        &self.attrs
    }

    pub(super) fn tree_parent(&self) -> &Once<TreeParent> {
        &self.tree_parent
    }

    pub(super) fn devnum(&self) -> Option<DevNum> {
        self.devnum
    }

    pub(super) fn state(&self) -> &Mutex<State> {
        &self.state
    }

    pub(super) fn glue_dirs(&self) -> &GlueDirs {
        &self.glue_dirs
    }

    pub(super) fn links(&self) -> &Mutex<Links> {
        &self.links
    }

    pub(super) fn devnode(&self) -> &Mutex<Option<DevNodeRequest>> {
        &self.devnode
    }
}

impl SysTreeEdit for DeviceBase {
    fn attach_child(&self, child: Arc<dyn SysObj>) -> Result<()> {
        let mut children = self.children.write();
        let name = child.name().clone();
        if children.contains_key(&name) {
            return Err(Error::NameConflict);
        }
        let weak: Weak<dyn SysBranchNode> = self.fields.weak_self().clone();
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
        let Some(parent) = self.fields.parent() else {
            return String::from(self.name().as_ref());
        };
        let mut path = String::from(parent.path().as_ref());
        if !parent.is_root() {
            path.push('/');
        }
        path.push_str(self.name());
        path
    }
}

impl core::fmt::Debug for DeviceBase {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DeviceBase")
            .field("name", self.name())
            .field("devnum", &self.devnum)
            .field("state", &*self.state.lock())
            .finish_non_exhaustive()
    }
}
