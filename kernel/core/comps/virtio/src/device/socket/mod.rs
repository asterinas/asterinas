// SPDX-License-Identifier: MPL-2.0

//! Basic virtio-vsock device support.
//!
//! This module implements the transmit and receive primitives of a virtio-vsock device, together
//! with the minimal queue management and callbacks required to drive the hardware. Higher layers
//! are responsible for protocol-level logic, such as interpreting received packet operations and
//! managing connection state.
//!
//! For a quick start, look up the device using the [`get_device`] function, then send packets via
//! the transmit queue using the [`SocketDevice::lock_tx`] method and receive packets via the
//! receive queue using the [`SocketDevice::lock_rx`] method. Notifications for newly received
//! packets happen via the callback specified in the [`SocketDevice::init_rx_callback`] method,
//! which is called in the bottom half.

mod buffer;
mod config;
pub mod device;
pub mod header;
pub mod packet;
pub mod queue;

use alloc::{
    collections::BTreeMap,
    string::String,
    sync::{Arc, Weak},
    vec::Vec,
};

use aster_softirq::Taskless;
use ostd::sync::{LocalIrqDisabled, SpinLock};
use spin::Once;

use crate::device::socket::device::SocketDevice;

/// The default virtio-vsock device name used by the kernel networking stack.
pub const DEVICE_NAME: &str = "Virtio-Vsock";

struct Component {
    devices: SpinLock<BTreeMap<String, Arc<SocketDevice>>>,
    rx_pending: SpinLock<Vec<Weak<SocketDevice>>, LocalIrqDisabled>,
    tx_pending: SpinLock<Vec<Weak<SocketDevice>>, LocalIrqDisabled>,
    event_pending: SpinLock<Vec<Weak<SocketDevice>>, LocalIrqDisabled>,
    rx_taskless: Arc<Taskless>,
    tx_taskless: Arc<Taskless>,
    event_taskless: Arc<Taskless>,
}

static COMPONENT: Once<Component> = Once::new();

/// Initializes the global virtio-vsock component state.
pub(crate) fn init() {
    buffer::init();
    COMPONENT.call_once(|| Component {
        devices: SpinLock::new(BTreeMap::new()),
        rx_pending: SpinLock::new(Vec::new()),
        tx_pending: SpinLock::new(Vec::new()),
        event_pending: SpinLock::new(Vec::new()),
        rx_taskless: Taskless::new(process_pending_rx),
        tx_taskless: Taskless::new(process_pending_tx),
        event_taskless: Taskless::new(process_pending_event),
    });
}

/// Registers a virtio-vsock device under `name`.
fn register_device(name: String, device: Arc<SocketDevice>) {
    let component = COMPONENT.get().unwrap();
    component.devices.lock().insert(name, device);
}

/// Returns the registered virtio-vsock device named `name`, if any.
pub fn get_device(name: &str) -> Option<Arc<SocketDevice>> {
    let component = COMPONENT.get().unwrap();
    component.devices.lock().get(name).cloned()
}

// #### Methods to schedule `Taskless`:

fn schedule_rx(device: &Weak<SocketDevice>) {
    let component = COMPONENT.get().unwrap();
    component.rx_pending.lock().push(device.clone());
    component.rx_taskless.schedule();
}

fn schedule_tx(device: &Weak<SocketDevice>) {
    let component = COMPONENT.get().unwrap();
    component.tx_pending.lock().push(device.clone());
    component.tx_taskless.schedule();
}

fn schedule_event(device: &Weak<SocketDevice>) {
    let component = COMPONENT.get().unwrap();
    component.event_pending.lock().push(device.clone());
    component.event_taskless.schedule();
}

// #### Methods used as `Taskless` callbacks:

fn process_pending_rx() {
    let component = COMPONENT.get().unwrap();
    let devices = take_pending(&component.rx_pending);

    for device in devices {
        if let Some(device) = device.upgrade() {
            device.process_rx();
        }
    }
}

fn process_pending_tx() {
    let component = COMPONENT.get().unwrap();
    let devices = take_pending(&component.tx_pending);

    for device in devices {
        if let Some(device) = device.upgrade() {
            device.lock_tx().free_processed_tx_buffers();
        }
    }
}

fn process_pending_event() {
    let component = COMPONENT.get().unwrap();
    let devices = take_pending(&component.event_pending);

    for device in devices {
        if let Some(device) = device.upgrade() {
            device.process_event();
        }
    }
}

fn take_pending(
    pending: &SpinLock<Vec<Weak<SocketDevice>>, LocalIrqDisabled>,
) -> Vec<Weak<SocketDevice>> {
    let mut pending = pending.lock();
    core::mem::take(&mut *pending)
}
