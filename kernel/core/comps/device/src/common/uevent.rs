// SPDX-License-Identifier: MPL-2.0

//! Building and delivering device uevents.
//!
//! [`emit`] collects the device's `KEY=VALUE` uevent entries and commits the
//! event through the kernel broadcast hook. [`show`] and [`store`] implement
//! the per-device sysfs `uevent` attribute: reading lists those entries, and
//! writing sends a synthetic event without changing the device.

use alloc::string::{String, ToString};
use core::fmt::Write;

use super::{AnyDevice, Error, Result, node::SysTreeEdit};
use crate::{
    hooks,
    uevent::{self, PreparedUevent, Uevent, UeventAction, UeventVars},
};

/// Returns the absolute sysfs path used in `DEVPATH`.
///
/// After teardown the parent is detached, so `tree_path()` may return only a
/// name. The leading slash keeps the path valid for a `change` sent after
/// `remove`.
fn event_path(dev: &dyn AnyDevice) -> String {
    let path = dev.base().tree_path();
    if path.starts_with('/') {
        return path;
    }
    let mut absolute = String::with_capacity(path.len() + 1);
    absolute.push('/');
    absolute.push_str(&path);
    absolute
}

/// Sends an explicit event with `extra` entries appended after the device
/// environment.
///
/// A device without a subsystem name does not generate an event. The device
/// itself is not modified, so a synthetic `remove` or `bind` only notifies
/// listeners. Environment and hook errors are returned to the caller.
pub(crate) fn emit(dev: &dyn AnyDevice, action: UeventAction, extra: &UeventVars) -> Result<()> {
    let subsystem = dev.subsystem();
    let Some(subsystem_name) = subsystem.name() else {
        return Ok(());
    };
    let devpath = event_path(dev);
    let mut vars = build_vars(dev, Some(action))?;
    for (key, value) in extra.iter() {
        vars.add(key, value)?;
    }

    PreparedUevent::new(action, devpath, subsystem_name.to_string(), vars).commit()
}

impl PreparedUevent {
    /// Collects the device's `KEY=VALUE` uevent entries for an automatic event.
    ///
    /// Returns `None` if the device has no subsystem name or the entries
    /// cannot be collected; the failure is logged because automatic events
    /// must not affect the device operation.
    pub(super) fn prepare(dev: &dyn AnyDevice, action: UeventAction) -> Option<Self> {
        let subsystem = dev.subsystem();
        let subsystem_name = subsystem.name()?;
        let devpath = event_path(dev);
        match build_vars(dev, Some(action)) {
            Ok(vars) => Some(Self::new(action, devpath, subsystem_name.to_string(), vars)),
            Err(error) => {
                ostd::debug!("cannot emit {action} for {}: {error}", dev.name());
                None
            }
        }
    }

    /// Validates the environment, allocates a sequence number, and broadcasts it.
    ///
    /// Environment and hook errors are returned to the caller.
    pub(super) fn commit(self) -> Result<()> {
        let event = Uevent::new(self)?;
        hooks::broadcast_uevent(&event)
    }
}

/// Sends the automatic event for a device-model operation.
///
/// These are `Add` on registration, `Remove` on unregistration, and
/// `Bind`/`Unbind` on driver binding. The operation must complete even when
/// the notification cannot be built or delivered, so failures are logged and
/// ignored.
pub(crate) fn emit_lifecycle(dev: &dyn AnyDevice, action: UeventAction) {
    if let Some(prepared) = PreparedUevent::prepare(dev, action)
        && let Err(error) = prepared.commit()
    {
        ostd::debug!("cannot emit {action} for {}: {error}", dev.name());
    }
}

/// Writes the device environment as `KEY=VALUE` lines.
///
/// Reading is a query: it does not broadcast an event or allocate a sequence
/// number, and it omits `ACTION`, `DEVPATH`, `SUBSYSTEM`, and `SEQNUM`.
/// Returns [`Error::NotAdded`] if the device has already been removed.
pub(super) fn show(dev: &dyn AnyDevice, writer: &mut dyn Write) -> Result<()> {
    let vars = build_vars(dev, None)?;
    if !dev.base().is_added() {
        return Err(Error::NotAdded);
    }
    for (key, value) in vars.iter() {
        writeln!(writer, "{key}={value}")?;
    }
    Ok(())
}

/// Handles a write to the sysfs `uevent` attribute as a synthetic request.
///
/// The write only notifies listeners; it does not add, remove, bind, or
/// unbind the device. Malformed syntax and a synthetic environment over
/// 64 entries or 2048 bytes produce [`Error::InvalidValue`] (`EINVAL`);
/// overflow after merging the device environment produces
/// [`Error::NoMemory`] (`ENOMEM`).
pub(super) fn store(dev: &dyn AnyDevice, text: &str) -> Result<()> {
    let (action, vars) = uevent::parse_synthetic(text)?;
    emit(dev, action, &vars)
}

/// Collects the device-owned `KEY=VALUE` entries.
///
/// `MAJOR`, `MINOR`, and `DEVNAME` appear for a device with a non-zero major
/// number; `DEVMODE` appears only for an explicit non-zero mode and encodes
/// the low nine permission bits in octal. `DEVTYPE` and `DRIVER` reflect the
/// state at construction time. Subsystem and device-type callbacks append
/// their entries afterwards, in that order. For `Unbind`, every `MODALIAS`
/// entry is removed after the callbacks run.
fn build_vars(dev: &dyn AnyDevice, action: Option<UeventAction>) -> Result<UeventVars> {
    let mut vars = UeventVars::new();
    if let Some(devnum) = dev.devnum()
        && devnum.id().major().get() != 0
    {
        vars.add("MAJOR", devnum.id().major().get())?;
        vars.add("MINOR", devnum.id().minor().get())?;
        let mode = {
            let node = dev.base().devnode().lock();
            let request = node.as_ref().ok_or(Error::NotAdded)?;
            vars.add("DEVNAME", &request.path)?;
            request.mode.map(|mode| mode & 0o777)
        };
        if let Some(mode) = mode {
            // `mode` has already been masked to the low nine permission
            // bits. An explicit policy that masks to zero is still `DEVMODE=0`.
            if mode == 0 {
                vars.add("DEVMODE", "0")?;
            } else {
                vars.add("DEVMODE", format_args!("0{mode:o}"))?;
            }
        }
    }
    if let Some(name) = dev.type_name() {
        vars.add("DEVTYPE", name)?;
    }
    if let Some(driver) = dev.driver_name() {
        vars.add("DRIVER", driver)?;
    }
    dev.append_uevent_vars(&mut vars)?;
    if action == Some(UeventAction::Unbind) {
        vars.remove("MODALIAS");
    }
    Ok(vars)
}
