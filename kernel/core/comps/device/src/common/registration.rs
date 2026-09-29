// SPDX-License-Identifier: MPL-2.0

//! The registration sequence: `add`, `remove`,
//! and the builder that every typed device is created through.

use alloc::{
    string::ToString,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::fmt::Write;

use aster_systree::SysObj;

use super::{AnyDevice, DeclaredParts, DeviceType, State, TreeParent};
use crate::{
    common::{
        Error, Result, Subsystem, SubsystemKind, SysStr,
        attr::{Attr, TyErasedAttr},
        devnum::{DEFAULT_DEVNODE_MODE, DevNum},
        node::{SysTreeEdit, add_link, remove_link},
        registry,
    },
    hooks::{self, DevNodeRequest},
};

/// Registers a device, publishing its sysfs directory and optional `/dev` node.
///
/// The device's sysfs attributes and links are available before its bus probes for a driver
/// or its class notifies observers.
/// Device-node creation is queued until the kernel hooks are installed.
///
/// On failure, registration is rolled back and the device cannot be added again.
pub fn add<D: AnyDevice + ?Sized>(dev: &Arc<D>) -> Result<()> {
    add_erased(dev.to_arc())
}

/// Unregisters a device, removing its sysfs directory and optional `/dev` node.
///
/// The device must have no child devices when removed.
/// If its children were created by a driver, unbind that driver from the device first.
pub fn remove<D: AnyDevice + ?Sized>(dev: &Arc<D>) -> Result<()> {
    let this = dev.to_arc();
    let base = this.base();
    {
        let mut state = base.state().lock();
        let State::Added(children) = &*state else {
            return Err(Error::NotAdded);
        };
        if !children.is_empty() {
            return Err(Error::HasChildren);
        }
        *state = State::Removed;
    }
    teardown(&this);
    Ok(())
}

/// A builder for a bus or class device.
///
/// Obtained from [`BusDevice::builder`] or [`ClassDevice::builder`].
/// The built device is not in the tree until [`add_device`] is called.
///
/// `H` is the subsystem handle, `P` the payload, and `D` the concrete Rust type of the device.
///
/// [`BusDevice::builder`]: crate::bus::BusDevice::builder
/// [`ClassDevice::builder`]: crate::class::ClassDevice::builder
/// [`add_device`]: crate::add_device
pub struct DeviceBuilder<H, P, D: 'static> {
    pub(crate) handle: H,
    pub(crate) payload: P,
    pub(crate) name: SysStr,
    pub(crate) parent: Option<Arc<dyn AnyDevice>>,
    pub(crate) devnum: Option<DevNum>,
    pub(super) dev_type: Option<&'static DeviceType<D>>,
    pub(super) attrs: &'static [Attr<D>],
}

impl<H, P, D: 'static> DeviceBuilder<H, P, D> {
    /// Sets the parent device; it must be registered before this device is.
    pub fn parent(mut self, parent: Arc<dyn AnyDevice>) -> Self {
        self.parent = Some(parent);
        self
    }

    /// Sets the device number and requests a `/dev` node when the device is registered.
    ///
    /// By default, the node's path relative to `/dev` is the device name
    /// with every `!` replaced by `/`.
    /// For example, `cciss!c0d0` becomes `/dev/cciss/c0d0`.
    ///
    /// [`DeviceType::devnode`] can override the path.
    /// For class devices, [`Class::devnode`] can provide a path
    /// if the device type does not provide one.
    ///
    /// [`Class::devnode`]: crate::class::Class::devnode
    pub fn devnum(mut self, devnum: DevNum) -> Self {
        self.devnum = Some(devnum);
        self
    }

    /// Gives the device a device type.
    pub fn dev_type(mut self, dev_type: &'static DeviceType<D>) -> Self {
        self.dev_type = Some(dev_type);
        self
    }

    /// Gives the device attributes of its own.
    pub fn attrs(mut self, attrs: &'static [Attr<D>]) -> Self {
        self.attrs = attrs;
        self
    }

    pub(crate) fn new(handle: H, name: SysStr, payload: P) -> Self {
        Self {
            handle,
            payload,
            name,
            parent: None,
            devnum: None,
            dev_type: None,
            attrs: &[],
        }
    }

    /// Returns the device type and attributes declared for the device.
    pub(crate) fn declared_parts(&self) -> DeclaredParts<D> {
        DeclaredParts {
            dev_type: self.dev_type,
            own_attrs: self.attrs,
        }
    }
}

fn add_erased(this: Arc<dyn AnyDevice>) -> Result<()> {
    let base = this.base();
    {
        let mut state = base.state().lock();
        if !matches!(*state, State::Initialized) {
            return Err(Error::AlreadyAdded);
        }
        *state = State::Preparing;
    }

    match add_steps(&this) {
        Ok(()) => {
            let mut state = base.state().lock();
            let State::Adding(children) = &mut *state else {
                unreachable!("a device being registered is in the adding state");
            };
            *state = State::Added(core::mem::take(children));
            Ok(())
        }
        Err(e) => {
            *base.state().lock() = State::Removed;
            teardown(&this);
            Err(e)
        }
    }
}

fn add_steps(this: &Arc<dyn AnyDevice>) -> Result<()> {
    let base = this.base();
    let subsystem = this.subsystem();

    // 1. Place the directory.
    place(this)?;

    // 2. Core attributes.
    let mut attrs = Vec::new();
    if base.devnum().is_some() {
        attrs.push(TyErasedAttr::from_dyn(&DEV_ATTR));
    }

    // 3. Subsystem, device type, and device attributes.
    attrs.extend(this.attr_groups());
    base.attrs().add(attrs)?;

    // 4. Links: `subsystem`, `device`, and the index entry.
    let path = base.tree_path();
    if let Some(dir) = subsystem.dir() {
        add_link(base, "subsystem", &SysObj::path(dir.as_ref()))?;
        base.links().lock().subsystem = true;
    }
    if let (SubsystemKind::Class, Some(parent)) = (subsystem.kind(), base.parent())
        && this.wants_device_link()
    {
        add_link(base, "device", &parent.base().tree_path())?;
        base.links().lock().device = true;
    }
    if let Some(index) = subsystem.index_dir() {
        add_link(index.as_ref(), base.name(), &path)?;
        base.links().lock().index = true;
    }

    // 5. Device number: `/sys/dev` entry and `/dev` node.
    if let Some(devnum) = base.devnum() {
        let index = registry::get().dev_index(devnum.kind());
        add_link(index.as_ref(), &devnum.to_string(), &path)?;
        base.links().lock().dev_index = true;
        let request = devnode_request(this.as_ref(), devnum);
        hooks::create_devnode(request.clone())?;
        *base.devnode().lock() = Some(request);
    }

    // No fallible registration steps remain. The path is now stable, and
    // callbacks can register children without a later rollback orphaning them.
    *base.state().lock() = State::Adding(Vec::new());

    // 6. Let the subsystem act.
    if let Some(ops) = subsystem.ops() {
        ops.on_added(this);
    }

    Ok(())
}

/// Undoes registration, tolerating steps that never happened.
fn teardown(this: &Arc<dyn AnyDevice>) {
    let base = this.base();
    let subsystem = this.subsystem();
    // Only links this device created are removed: an index entry under the
    // same name may belong to another device whose name this one clashed with.
    let links = core::mem::take(&mut *base.links().lock());

    // 6. Let the subsystem act.
    if let Some(ops) = subsystem.ops() {
        ops.on_removed(this);
    }

    // 5. Device number.
    if let Some(devnum) = base.devnum() {
        // Taken in its own statement: the guard of an `if let` scrutinee lives
        // to the end of the block, which would hold this device's `devnode`
        // mutex across the hook and put the hook queue underneath it.
        let request = base.devnode().lock().take();
        if let Some(request) = request {
            let _ = hooks::delete_devnode(&request);
        }
        if links.dev_index {
            remove_link(
                registry::get().dev_index(devnum.kind()).as_ref(),
                &devnum.to_string(),
            );
        }
    }

    // 4. Links.
    if links.index
        && let Some(index) = subsystem.index_dir()
    {
        remove_link(index.as_ref(), base.name());
    }
    if links.device {
        remove_link(base, "device");
    }
    if links.subsystem {
        remove_link(base, "subsystem");
    }

    // 3. and 2. Attributes are dropped with the directory.
    // 1. Detach the directory, and any glue directory left empty.
    if let Some(tree_parent) = base.tree_parent().get() {
        tree_parent.with_edit(|parent| {
            let _ = parent.detach_child(base.name());
        });
        let class = subsystem.name().unwrap_or_default();
        match (subsystem.kind(), base.parent()) {
            (SubsystemKind::Class, Some(parent)) if uses_glue_dir(&subsystem, parent) => {
                parent
                    .base()
                    .glue_dirs()
                    .drop_if_empty(class, parent.base());
            }
            (SubsystemKind::Class, None) => registry::get().drop_virtual_glue_dir_if_empty(class),
            _ => {}
        }
    }

    // Release the parent only after all cleanup has finished.
    if let Some(parent) = base.parent() {
        let mut state = parent.base().state().lock();
        if let State::Adding(children) | State::Added(children) = &mut *state {
            let this = Arc::downgrade(this);
            children.retain(|child| !Weak::ptr_eq(child, &this));
        }
    }
}

/// Decides which directory a device's directory goes into, and attaches it there.
///
/// - A class device under a class device sits directly inside it,
///   unless the class sets `KEEPS_GLUE_DIR`,
///   in which case it sits in a glue directory named after its class inside the parent.
/// - A class device under any other device sits in a glue directory named after its class,
///   created inside the parent.
/// - A class device with no parent sits in a glue directory under `/sys/devices/virtual`.
/// - A bus device or a bare device sits directly under its parent,
///   or at the top of `/sys/devices` if it has none.
fn place(dev: &Arc<dyn AnyDevice>) -> Result<()> {
    let parent = dev.base().parent();
    if let Some(parent) = parent {
        // Checking the parent and recording the child must be one operation
        // with respect to parent removal. Rollback releases this reservation.
        let mut state = parent.base().state().lock();
        let (State::Adding(children) | State::Added(children)) = &mut *state else {
            return Err(Error::ParentNotAdded);
        };
        children.push(Arc::downgrade(dev));
    }

    let subsystem = dev.subsystem();
    let class = subsystem.name().unwrap_or_default();
    let child: Arc<dyn SysObj> = dev.clone();
    let tree_parent = match (subsystem.kind(), parent) {
        (SubsystemKind::Class, Some(parent)) => {
            if uses_glue_dir(&subsystem, parent) {
                let dir = parent
                    .base()
                    .glue_dirs()
                    .attach_into(class, parent.base(), child)?;
                TreeParent::Dir(Arc::downgrade(&dir))
            } else {
                parent.base().attach_child(child)?;
                TreeParent::Device(Arc::downgrade(parent))
            }
        }
        (SubsystemKind::Class, None) => {
            let dir = registry::get().attach_into_virtual_glue_dir(class, child)?;
            TreeParent::Dir(Arc::downgrade(&dir))
        }
        (_, Some(parent)) => {
            parent.base().attach_child(child)?;
            TreeParent::Device(Arc::downgrade(parent))
        }
        (_, None) => {
            let root = registry::get().devices_root();
            root.attach_child(child)?;
            TreeParent::Dir(Arc::downgrade(root))
        }
    };
    dev.base().tree_parent().call_once(|| tree_parent);
    Ok(())
}

/// Returns whether a class device under `parent` is placed in a glue directory,
/// as opposed to directly inside a class-device parent.
fn uses_glue_dir(subsystem: &Subsystem, parent: &Arc<dyn AnyDevice>) -> bool {
    let sits_inside_parent = parent.subsystem().is_class();
    !sits_inside_parent || subsystem.keeps_glue_dir()
}

/// Builds a `/dev` node creation request using the device's path and permission overrides.
fn devnode_request(dev: &dyn AnyDevice, devnum: DevNum) -> DevNodeRequest {
    let over = dev.devnode_override().unwrap_or_default();
    DevNodeRequest {
        devnum,
        path: over
            .path
            .unwrap_or_else(|| SysStr::from(dev.base().name().replace('!', "/"))),
        mode: over.mode.unwrap_or(DEFAULT_DEVNODE_MODE),
    }
}

const DEV_ATTR: Attr<dyn AnyDevice> = Attr::ro("dev", show_dev);

fn show_dev(dev: &dyn AnyDevice, w: &mut dyn Write) -> Result<()> {
    let devnum = dev.devnum().ok_or(Error::NoDevNum)?;
    writeln!(w, "{}", devnum)?;
    Ok(())
}
