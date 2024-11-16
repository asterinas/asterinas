// SPDX-License-Identifier: MPL-2.0

mod ext;
mod init;
mod poll;

pub use ext::IfaceEx;
pub use init::{init, IFACES};
pub use poll::lazy_init;

pub type Iface = dyn aster_bigtcp::iface::Iface<ext::IfaceExt>;
pub type BoundTcpSocket = aster_bigtcp::socket::BoundTcpSocket<ext::IfaceExt>;
pub type BoundUdpSocket = aster_bigtcp::socket::BoundUdpSocket<ext::IfaceExt>;
