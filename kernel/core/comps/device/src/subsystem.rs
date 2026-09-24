// SPDX-License-Identifier: MPL-2.0

//! The subsystem a device belongs to and the operations shared by buses and classes.

use alloc::sync::Arc;

use crate::{device::AnyDevice, node::Dir};

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

/// The subsystem that owns a device.
/// A device has exactly one.
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

    /// Returns the directory that lists the device: `/sys/bus/<bus>/devices` or `/sys/class/<class>`.
    pub(crate) fn index_dir(&self) -> Option<Arc<Dir>> {
        self.ops.as_ref().map(|ops| ops.index_dir())
    }

    /// Returns whether a class device under a class-device parent still gets a glue directory.
    pub(crate) fn keeps_glue_dir(&self) -> bool {
        self.ops.as_ref().is_some_and(|ops| ops.keeps_glue_dir())
    }
}

/// What the registration sequence needs from a bus or class handle.
///
/// Implemented by [`BusHandle`](crate::BusHandle) and [`ClassHandle`](crate::ClassHandle),
/// and reachable only through [`Subsystem`], whose field is private,
/// so that the callbacks cannot be invoked from outside the crate.
pub(crate) trait SubsystemOps: Send + Sync + 'static {
    /// Returns the bus or class name.
    fn name(&self) -> &'static str;

    /// Returns the `/sys/bus/<name>` or `/sys/class/<name>` directory.
    fn dir(&self) -> Arc<Dir>;

    /// Returns the directory that lists the subsystem's devices.
    fn index_dir(&self) -> Arc<Dir>;

    /// Returns whether a class device under a class-device parent still gets a glue directory
    /// (always false for a bus).
    fn keeps_glue_dir(&self) -> bool;

    /// Records a device that has just been registered and acts on it:
    /// a bus probes for a driver, a class notifies its interfaces.
    fn on_added(&self, dev: &Arc<dyn AnyDevice>);

    /// Forgets a device that is being removed: a bus unbinds it, a class notifies its interfaces.
    fn on_removed(&self, dev: &Arc<dyn AnyDevice>);
}
