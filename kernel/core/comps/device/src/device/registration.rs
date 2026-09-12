// SPDX-License-Identifier: MPL-2.0

//! The registration sequence: `add`, `remove`, and the builder that every
//! typed device is created through.

use alloc::{
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use core::fmt::Write;

use aster_systree::SysObj;

use super::{
    AnyDevice, DeclaredParts, DevNodeSpec, DeviceType, State, Subsystem, SubsystemKind, TreeParent,
};
use crate::{
    Error, Result, SysStr,
    attr::{Attr, TyErasedAttr},
    devnum::{DEFAULT_DEVNODE_MODE, DevNodeRequest, DevNum},
    hooks,
    node::{SysTreeEdit, add_link, remove_link},
    registry,
    uevent::{Uevent, UeventAction, UeventVars},
};

/// Whether [`teardown`] announces the removal to user space.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Teardown {
    /// A registered device is being removed: send the `remove` uevent.
    Announced,
    /// A registration is being undone: user space never saw the device.
    Silent,
}

/// Builds a bus or class device.
///
/// Obtained from [`BusDevice::builder`](super::BusDevice::builder) or
/// [`ClassDevice::builder`](super::ClassDevice::builder). The built device is
/// not in the tree until [`add`] is called.
///
/// `H` is the subsystem handle, `P` the payload, and `D` the device type the
/// attributes apply to.
pub struct DeviceBuilder<H, P, D: ?Sized + 'static> {
    pub(super) handle: H,
    pub(super) payload: P,
    pub(super) name: SysStr,
    pub(super) parent: Option<Arc<dyn AnyDevice>>,
    pub(super) devnum: Option<DevNum>,
    pub(super) dev_type: Option<&'static DeviceType<D>>,
    pub(super) attrs: &'static [Attr<D>],
}

impl<H, P, D: ?Sized + 'static> DeviceBuilder<H, P, D> {
    pub(super) fn new(handle: H, name: SysStr, payload: P) -> Self {
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

    /// Sets the parent device; it must be registered before this device is.
    pub fn parent(mut self, parent: Arc<dyn AnyDevice>) -> Self {
        self.parent = Some(parent);
        self
    }

    /// Gives the device a device number, and so a `/dev` node.
    pub fn devnum(mut self, devnum: DevNum) -> Self {
        self.devnum = Some(devnum);
        self
    }

    /// Gives the device a type.
    pub fn dev_type(mut self, dev_type: &'static DeviceType<D>) -> Self {
        self.dev_type = Some(dev_type);
        self
    }

    /// Gives the device attributes of its own.
    pub fn attrs(mut self, attrs: &'static [Attr<D>]) -> Self {
        self.attrs = attrs;
        self
    }

    /// Returns the per-type parts the built device will carry.
    pub(super) fn declared_parts(&self) -> DeclaredParts<D> {
        DeclaredParts {
            dev_type: self.dev_type,
            own_attrs: self.attrs,
        }
    }
}

/// Builds the `uevent` attribute content: the device-specific variables.
fn dev_uevent_vars(dev: &dyn AnyDevice) -> UeventVars {
    let mut vars = UeventVars::new();
    if let Some(devnum) = dev.base().devnum() {
        vars.add("MAJOR", devnum.id().major().get());
        vars.add("MINOR", devnum.id().minor().get());
        // The node the device was registered with, so that the variables and
        // the file in `/dev` agree even if a `devnode` callback is not pure.
        // Before that node exists, the callbacks are asked again.
        let node = dev
            .base()
            .devnode_spec()
            .unwrap_or_else(|| devnode_spec(dev, devnum));
        vars.add("DEVNAME", &node.request.path);
        // Linux prints the mode only when a `devnode` callback set one, with
        // `%#03o` over the permission bits, e.g. `0666`.
        if node.has_mode {
            vars.add("DEVMODE", format_args!("0{:o}", node.request.mode & 0o777));
        }
    }
    if let Some(type_name) = dev.type_name() {
        vars.add("DEVTYPE", type_name);
    }
    if let Some(driver) = dev.driver_name() {
        vars.add("DRIVER", driver);
    }
    dev.subsystem_uevent(&mut vars);
    vars
}

/// Computes the `/dev` node a device gets: the type's override first, then
/// the subsystem's, then the defaults.
///
/// Without an override the path is the device name with every `!` replaced
/// by `/`, as Linux's `device_get_devnode` does in the branch it reaches when
/// neither the type nor the subsystem named a node, so that a name such as
/// `cciss!c0d0` becomes the node `cciss/c0d0`.
///
/// The result is computed once, in step 5 of [`add`], and every later use
/// (the uevent variables, the deletion of the node) reads that one value.
fn devnode_spec(dev: &dyn AnyDevice, devnum: DevNum) -> DevNodeSpec {
    let over = dev.devnode_override();
    let has_mode = over.as_ref().is_some_and(|o| o.mode.is_some());
    let over = over.unwrap_or_default();
    DevNodeSpec {
        request: DevNodeRequest {
            devnum,
            path: over
                .path
                .unwrap_or_else(|| SysStr::from(dev.base().name().replace('!', "/"))),
            mode: over.mode.unwrap_or(DEFAULT_DEVNODE_MODE),
        },
        has_mode,
    }
}

/// Builds and broadcasts a uevent for a device with a subsystem.
fn emit_uevent(dev: &dyn AnyDevice, action: UeventAction) {
    let Some(subsystem) = dev.subsystem().name().map(String::from) else {
        // Devices without a subsystem send no events, as in Linux.
        return;
    };
    let event = Uevent::new(
        action,
        dev.base().tree_path(),
        subsystem,
        dev_uevent_vars(dev),
    );
    hooks::broadcast_uevent(event);
}

const CORE_ATTRS: &[Attr<dyn AnyDevice>] = &[Attr::rw("uevent", show_uevent, store_uevent)];
const DEV_ATTR: Attr<dyn AnyDevice> = Attr::ro("dev", show_dev);

fn show_uevent(dev: &dyn AnyDevice, w: &mut dyn Write) -> Result<()> {
    dev_uevent_vars(dev).write_lines(w)?;
    Ok(())
}

fn store_uevent(dev: &dyn AnyDevice, value: &str) -> Result<()> {
    let action: UeventAction = value.parse()?;
    emit_uevent(dev, action);
    Ok(())
}

fn show_dev(dev: &dyn AnyDevice, w: &mut dyn Write) -> Result<()> {
    let devnum = dev.base().devnum().ok_or(Error::NoDevNum)?;
    writeln!(w, "{}", devnum)?;
    Ok(())
}

/// Returns whether a class device under `parent` is placed in a glue
/// directory, as opposed to directly inside a class-device parent.
fn uses_glue_dir(subsystem: &Subsystem, parent: &Arc<dyn AnyDevice>) -> bool {
    let sits_inside_parent = parent.is_class_device();
    !sits_inside_parent || subsystem.keeps_glue_dir()
}

/// Decides which directory a device's directory goes into, and attaches it
/// there.
///
/// - A class device under a class device sits directly inside it.
/// - A class device under any other device sits in a glue directory named
///   after its class, created inside the parent.
/// - A class device with no parent sits in a glue directory under
///   `/sys/devices/virtual`.
/// - A bus device or a bare device sits directly under its parent, or at the
///   top of `/sys/devices` if it has none.
fn place(dev: &Arc<dyn AnyDevice>) -> Result<()> {
    let parent = dev.base().parent();
    if let Some(parent) = parent
        && !parent.base().is_added()
    {
        return Err(Error::ParentNotAdded);
    }

    let subsystem = dev.subsystem();
    let class = subsystem.name().unwrap_or_default();
    let child: Arc<dyn SysObj> = dev.clone();
    let tree_parent = match (subsystem.kind(), parent) {
        (SubsystemKind::Class, Some(parent)) => {
            if uses_glue_dir(&subsystem, parent) {
                let dir = parent
                    .base()
                    .glue_dirs
                    .attach_into(class, parent.base(), child)?;
                TreeParent::Dir(Arc::downgrade(&dir))
            } else {
                parent.base().attach_child(child)?;
                TreeParent::Device(Arc::downgrade(parent))
            }
        }
        (SubsystemKind::Class, None) => {
            let dir = registry().attach_into_virtual_glue_dir(class, child)?;
            TreeParent::Dir(Arc::downgrade(&dir))
        }
        (_, Some(parent)) => {
            parent.base().attach_child(child)?;
            TreeParent::Device(Arc::downgrade(parent))
        }
        (_, None) => {
            let root = registry().devices_root();
            root.attach_child(child)?;
            TreeParent::Dir(Arc::downgrade(root))
        }
    };
    dev.base().tree_parent.call_once(|| tree_parent);
    Ok(())
}

/// Registers a device: the Asterinas counterpart of Linux's `device_add`.
///
/// The steps, in order: place the directory, add the core attributes, add the
/// attributes of the subsystem, the type, and the device, create the
/// `subsystem`, `device`, and index symlinks, publish the device number and
/// the `/dev` node, announce the device, let the subsystem act (a bus probes
/// for a driver; a class notifies its interfaces), and link the device into
/// its parent. On failure everything done so far is undone.
///
/// A failed registration is final: the device ends up removed and cannot be
/// added again.
pub fn add<D: AnyDevice + ?Sized>(dev: &Arc<D>) -> Result<()> {
    add_erased(dev.to_arc())
}

fn add_erased(this: Arc<dyn AnyDevice>) -> Result<()> {
    let base = this.base();
    if base.name().is_empty() || base.name().contains('/') || base.name().contains('\0') {
        return Err(Error::InvalidName);
    }
    {
        let mut state = base.state.lock();
        if *state != State::Initialized {
            return Err(Error::AlreadyAdded);
        }
        *state = State::Adding;
    }

    match add_steps(&this) {
        Ok(()) => {
            *base.state.lock() = State::Added;
            Ok(())
        }
        Err(e) => {
            teardown(&this, Teardown::Silent);
            *base.state.lock() = State::Removed;
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
    let mut attrs: Vec<TyErasedAttr> = CORE_ATTRS.iter().map(TyErasedAttr::from_dyn).collect();
    if base.devnum().is_some() {
        attrs.push(TyErasedAttr::from_dyn(&DEV_ATTR));
    }
    // 3. Subsystem, type, and device attributes.
    attrs.extend(this.attr_groups());
    base.attrs.add(attrs)?;

    // 4. Links: `subsystem`, `device`, and the index entry.
    let path = base.tree_path();
    if let Some(dir) = subsystem.dir() {
        add_link(base, "subsystem", &SysObj::path(dir.as_ref()))?;
        base.links.lock().subsystem = true;
    }
    if let (SubsystemKind::Class, Some(parent)) = (subsystem.kind(), base.parent())
        && this.wants_device_link()
    {
        add_link(base, "device", &parent.base().tree_path())?;
        base.links.lock().device = true;
    }
    if let Some(index) = subsystem.index_dir() {
        add_link(index.as_ref(), base.name(), &path)?;
        base.links.lock().index = true;
    }

    // 5. Device number: `/sys/dev` entry and `/dev` node.
    if let Some(devnum) = base.devnum() {
        let index = registry().dev_index(devnum.kind());
        add_link(index.as_ref(), &devnum.to_string(), &path)?;
        base.links.lock().dev_index = true;
        let spec = devnode_spec(this.as_ref(), devnum);
        hooks::create_devnode(spec.request.clone())?;
        *base.devnode.lock() = Some(spec);
    }

    // 6. Announce.
    emit_uevent(this.as_ref(), UeventAction::Add);

    // 7. Let the subsystem act.
    if let Some(ops) = subsystem.ops() {
        ops.on_added(this);
    }

    // 8. Link into the parent.
    if let Some(parent) = base.parent() {
        parent
            .base()
            .child_devices
            .lock()
            .push(Arc::downgrade(this));
    }
    Ok(())
}

/// Unregisters a device: the counterpart of Linux's `device_del`.
///
/// Unlike Linux, a device with registered child devices cannot be removed;
/// remove the children first. A bound bus device is unbound on the way, but
/// since a driver's `remove` is usually what deletes the children it created,
/// the caller's rule is: unbind first, then remove.
pub fn remove<D: AnyDevice + ?Sized>(dev: &Arc<D>) -> Result<()> {
    let this = dev.to_arc();
    let base = this.base();
    {
        let mut state = base.state.lock();
        if *state != State::Added {
            return Err(Error::NotAdded);
        }
        if !base.child_devices().is_empty() {
            return Err(Error::HasChildren);
        }
        *state = State::Removed;
    }
    teardown(&this, Teardown::Announced);
    Ok(())
}

/// Undoes registration, tolerating steps that never happened.
fn teardown(this: &Arc<dyn AnyDevice>, mode: Teardown) {
    let base = this.base();
    let subsystem = this.subsystem();
    // Only links this device created are removed: an index entry under the
    // same name may belong to another device whose name this one clashed with.
    let links = core::mem::take(&mut *base.links.lock());

    // 8. Unlink from the parent.
    if let Some(parent) = base.parent() {
        parent
            .base()
            .child_devices
            .lock()
            .retain(|w| w.upgrade().is_some_and(|d| !Arc::ptr_eq(&d, this)));
    }
    // 7. Let the subsystem act.
    if let Some(ops) = subsystem.ops() {
        ops.on_removed(this);
    }
    // 6. Announce.
    if mode == Teardown::Announced {
        emit_uevent(this.as_ref(), UeventAction::Remove);
    }
    // 5. Device number.
    if let Some(devnum) = base.devnum() {
        // Taken in its own statement: the guard of an `if let` scrutinee lives
        // to the end of the block, which would hold this device's `devnode`
        // mutex across the hook and put the hook queue underneath it.
        let spec = base.devnode.lock().take();
        if let Some(spec) = spec {
            let _ = hooks::delete_devnode(&spec.request);
        }
        if links.dev_index {
            remove_link(
                registry().dev_index(devnum.kind()).as_ref(),
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
    let Some(tree_parent) = base.tree_parent.get() else {
        return;
    };
    tree_parent.with_edit(|parent| {
        let _ = parent.detach_child(base.name());
    });
    let class = subsystem.name().unwrap_or_default();
    match (subsystem.kind(), base.parent()) {
        (SubsystemKind::Class, Some(parent)) if uses_glue_dir(&subsystem, parent) => {
            parent.base().glue_dirs.drop_if_empty(class, parent.base());
        }
        (SubsystemKind::Class, None) => registry().drop_virtual_glue_dir_if_empty(class),
        _ => {}
    }
}
