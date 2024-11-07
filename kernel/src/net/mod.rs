// SPDX-License-Identifier: MPL-2.0

use alloc::ffi::CString;

use spin::{Once, RwLock};

pub mod iface;
pub mod socket;

pub static HOSTNAME: Once<RwLock<CString>> = Once::new();

pub fn init() {
    iface::init();
    socket::vsock::init();
    HOSTNAME.call_once(|| RwLock::new(CString::new("(none)").unwrap()));
}

/// Lazy init should be called after spawning init thread.
pub fn lazy_init() {
    iface::lazy_init();
}
