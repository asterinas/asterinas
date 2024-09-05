// SPDX-License-Identifier: MPL-2.0

pub mod iface;
pub mod socket;

pub fn init() {
    iface::init();
    socket::vsock::init();
}

/// Lazy init should be called after spawning init thread.
pub fn lazy_init() {
    iface::lazy_init();
}
