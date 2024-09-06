// SPDX-License-Identifier: MPL-2.0

mod ext;
mod init;
mod poll;

pub use init::{init, IFACES};
pub use poll::{lazy_init, poll_ifaces};

pub type Iface = dyn aster_bigtcp::iface::Iface<ext::IfaceExt>;
pub type AnyBoundSocket = aster_bigtcp::socket::AnyBoundSocket<ext::IfaceExt>;
