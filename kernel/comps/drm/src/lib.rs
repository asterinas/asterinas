// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;
#[macro_use]
extern crate ostd_pod;

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "drm: "
    };
}

mod device;
mod geometry;
mod simpledrm;

use alloc::{sync::Arc, vec::Vec};

use aster_framebuffer::FRAMEBUFFER;
use component::{ComponentInitError, init_component};
pub use device::{DrmDevice, DrmDeviceCapFlags, DrmDeviceCaps, DrmFeatures};
pub use geometry::DrmRectU32;
use ostd::sync::Mutex;
use spin::Once;

use crate::simpledrm::SimpleDrmDevice;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrmError {
    /// Generic invalid argument or state.
    Invalid,
    /// Device or Object not found.
    NotFound,
    /// Operation not supported by this driver / device.
    NotSupported,
    /// Resource temporarily unavailable (busy, in use).
    Busy,
    /// Permission or access violation.
    PermissionDenied,
    /// Memory allocation or mapping failure.
    NoMemory,
}

pub fn register_drm_device(device: Arc<dyn DrmDevice>) {
    let component = COMPONENT
        .get()
        .expect("aster-drm component not initialized");

    component.drm_devices.lock().push(device);
}

pub fn registered_drm_devices() -> Vec<Arc<dyn DrmDevice>> {
    let component = COMPONENT
        .get()
        .expect("aster-drm component not initialized");

    component.drm_devices.lock().clone()
}

pub fn unregister_drm_device(device: &Arc<dyn DrmDevice>) -> Result<Arc<dyn DrmDevice>, DrmError> {
    let component = COMPONENT
        .get()
        .expect("aster-drm component not initialized");

    let mut devices = component.drm_devices.lock();
    if let Some(pos) = devices.iter().position(|d| Arc::ptr_eq(d, device)) {
        Ok(devices.remove(pos))
    } else {
        Err(DrmError::NotFound)
    }
}

static COMPONENT: Once<Component> = Once::new();

#[init_component]
fn component_init() -> Result<(), ComponentInitError> {
    let component = Component::init()?;
    COMPONENT.call_once(|| component);

    if FRAMEBUFFER.get().is_some() {
        let device = Arc::new(SimpleDrmDevice::new());
        register_drm_device(device);
    }

    Ok(())
}

#[derive(Debug)]
struct Component {
    drm_devices: Mutex<Vec<Arc<dyn DrmDevice>>>,
}

impl Component {
    fn init() -> Result<Self, ComponentInitError> {
        Ok(Self {
            drm_devices: Mutex::new(Vec::new()),
        })
    }
}
