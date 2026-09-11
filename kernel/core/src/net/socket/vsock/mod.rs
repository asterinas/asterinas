// SPDX-License-Identifier: MPL-2.0

//! This module defines vsock sockets.
//!
//! The vsock address family facilitates communication between virtual machines and the host they
//! are running on. This address family is used by guest agents and hypervisor services that need a
//! communications channel that is independent of virtual machine network configuration.
//!
//! The implementation is organized into three layers:
//! - The [_device layer_](`aster_virtio::device::socket`) provides the basic packet transmit and
//!   receive primitives.
//! - The [_transport layer_](`self::transport`) implements protocol logic such as connection and
//!   listener management.
//! - The [_socket layer_](`self::stream`) builds the Linux-compatible socket interface used by
//!   userspace-facing system calls.
//!

mod addr;
mod stream;
mod transport;

pub(crate) use addr::{VMADDR_CID_HOST, VsockSocketAddr};
use aster_virtio::device::socket::header::VirtioVsockHdr;
pub(crate) use stream::VsockStreamSocket;

use crate::prelude::Result;

pub(in crate::net) fn init() {
    transport::init();
}

/// Rejects using the host backend alongside an active virtio-vsock frontend.
pub(crate) fn ensure_vhost_backend() -> Result<()> {
    transport::ensure_vhost_backend()
}

/// Dispatches a packet read by an active vhost-vsock backend.
pub(crate) fn handle_vhost_packet(header: VirtioVsockHdr, payload: &[u8]) -> Result<()> {
    transport::handle_vhost_packet(header, payload)
}

/// Resets the connections of a guest after its backend stops routing packets.
///
/// The caller must not hold a backend lock and must prevent the CID from being reused until
/// this operation completes.
pub(crate) fn reset_vhost_connections(cid: u32) {
    transport::reset_vhost_connections(cid);
}

/// Wakes senders after the backend releases space in its outgoing packet queue.
///
/// The caller must not hold a backend lock.
pub(crate) fn notify_vhost_writable(cid: u32) {
    transport::notify_vhost_writable(cid);
}
