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

pub use addr::VsockSocketAddr;
pub use stream::VsockStreamSocket;

pub(in crate::net) fn init() {
    transport::init();
}
