// SPDX-License-Identifier: MPL-2.0

//! The Direct Rendering Manager (DRM) core framework of Asterinas.

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

pub mod device;

use alloc::{sync::Arc, vec::Vec};

use component::{ComponentInitError, init_component};
use ostd::sync::Mutex;
use spin::Once;

use crate::device::DrmDevice;

static COMPONENT: Once<Component> = Once::new();

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

#[init_component]
fn component_init() -> Result<(), ComponentInitError> {
    let component = Component::init()?;
    COMPONENT.call_once(|| component);

    Ok(())
}

/// Rectangles are checked by their right/bottom edges:
///
/// (x, y)        width        right = x + width
///    +-------------------------+
///    |                         |
///    |                         | height
///    |                         |
///    +-------------------------+
///                            bottom = y + height
///
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DrmRect {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

impl DrmRect {
    pub fn new(x: u32, y: u32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    pub fn x(&self) -> u32 {
        self.x
    }

    pub fn y(&self) -> u32 {
        self.y
    }

    pub fn width(&self) -> u32 {
        self.w
    }

    pub fn height(&self) -> u32 {
        self.h
    }

    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    pub fn right(&self) -> Option<u32> {
        self.x.checked_add(self.w)
    }

    pub fn bottom(&self) -> Option<u32> {
        self.y.checked_add(self.h)
    }

    /// Returns whether the point is inside the rectangle.
    ///
    /// The left/top edges are inclusive and the right/bottom edges are exclusive.
    pub fn contains_point(&self, x: u32, y: u32) -> bool {
        let Some(right) = self.right() else {
            return false;
        };
        let Some(bottom) = self.bottom() else {
            return false;
        };

        self.x <= x && x < right && self.y <= y && y < bottom
    }

    /// Returns whether `other` is fully contained within `self`.
    pub fn contains_rect(&self, other: &Self) -> bool {
        let Some(self_right) = self.right() else {
            return false;
        };
        let Some(self_bottom) = self.bottom() else {
            return false;
        };
        let Some(other_right) = other.right() else {
            return false;
        };
        let Some(other_bottom) = other.bottom() else {
            return false;
        };

        self.x <= other.x
            && self.y <= other.y
            && other_right <= self_right
            && other_bottom <= self_bottom
    }

    /// Returns whether the rectangle size is within the given inclusive limits.
    pub fn is_size_within(
        &self,
        min_width: u32,
        max_width: u32,
        min_height: u32,
        max_height: u32,
    ) -> bool {
        min_width <= self.w && self.w <= max_width && min_height <= self.h && self.h <= max_height
    }

    pub fn set_x(&mut self, x: u32) {
        self.x = x;
    }

    pub fn set_y(&mut self, y: u32) {
        self.y = y;
    }

    pub fn set_width(&mut self, w: u32) {
        self.w = w;
    }

    pub fn set_height(&mut self, h: u32) {
        self.h = h;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrmError {
    /// Generic invalid argument or state
    Invalid,
    /// Object not found (CRTC / FB / GEM handle / connector, etc.)
    NotFound,
    /// Operation not supported by this driver / device
    NotSupported,
    /// Operation is recognized but not implemented by this driver / device.
    FunctionNotImplemented,
    /// Resource temporarily unavailable (busy, in use)
    Busy,
    /// Permission or access violation
    PermissionDenied,
    /// Bad userspace address
    BadAddress,
    /// Memory allocation or mapping failure
    NoMemory,
    /// Resource already exist.
    AlreadyExist,
    /// Ioctl not found.
    IoctlNotFound,
}

impl From<DrmError> for ComponentInitError {
    fn from(error: DrmError) -> Self {
        match error {
            DrmError::AlreadyExist => {
                ostd::warn!("The device already registered")
            }
            _ => {}
        }
        ComponentInitError::Unknown
    }
}
