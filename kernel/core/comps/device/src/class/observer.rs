// SPDX-License-Identifier: MPL-2.0

//! Notifications of class membership changes.

use alloc::sync::Arc;

use super::{Class, ClassDevice};

/// An observer of devices joining or leaving a class.
///
/// An observer is notified when devices join or leave the class
/// through [`on_device_added`](Self::on_device_added) and [`on_device_removed`](Self::on_device_removed), respectively.
/// When the observer is registered or unregistered,
/// the corresponding callback is also called for every device currently in the class.
/// Each device is announced once, regardless of whether it or the observer is registered first.
///
/// Callbacks are serialized with changes to this class's devices and observers.
/// To avoid deadlocks, callbacks must not add or remove devices in this class,
/// register or unregister observers of this class,
/// bind or unbind bus devices, or read or write driver attributes.
/// Queue such work for another thread and return without waiting for it.
///
/// Typical uses include:
///
/// - **Partition discovery.**
///   An observer of the `block` class can schedule disk scans and registration of partition devices.
/// - **Console selection.**
///   An observer of the `tty` class can track available terminals for console selection.
pub trait ClassObserver<C: Class>: Send + Sync + 'static {
    /// Handles a device joining the class or being announced when this observer is registered.
    fn on_device_added(&self, dev: &Arc<ClassDevice<C>>);

    /// Handles a device leaving the class or this observer being unregistered.
    fn on_device_removed(&self, _dev: &Arc<ClassDevice<C>>) {}
}
