// SPDX-License-Identifier: MPL-2.0

mod ext;
mod init;
mod poll;
mod sched;

pub use init::{init, IFACES};
pub use poll::lazy_init;

pub type Iface = dyn aster_bigtcp::iface::Iface<ext::Ext>;
pub type BoundTcpSocket = aster_bigtcp::socket::BoundTcpSocket<ext::Ext>;
pub type BoundUdpSocket = aster_bigtcp::socket::BoundUdpSocket<ext::Ext>;
