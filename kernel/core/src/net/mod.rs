// SPDX-License-Identifier: MPL-2.0

pub(crate) mod iface;
pub(crate) mod socket;
pub(crate) mod uts_ns;

pub(crate) fn init() {
    iface::init();
    socket::netlink::init();
    socket::vsock::init();
}

/// Lazy init should be called after spawning init thread.
pub(crate) fn init_in_first_kthread() {
    iface::init_in_first_kthread();
}
