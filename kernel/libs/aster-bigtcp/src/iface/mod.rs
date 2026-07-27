// SPDX-License-Identifier: MPL-2.0

mod common;
#[expect(clippy::module_inception)]
mod iface;
mod packet_slice;
mod phy;
mod poll;
mod poll_iface;
mod port;
mod sched;
mod time;
mod wire;

const IFNAMESIZE: usize = 16;
pub type InterfaceName = aster_util::fixed_str::FixedCStr<IFNAMESIZE>;

pub use common::{BoundPort, BoundTcpPort, BoundUdpPort, InterfaceFlags, InterfaceType};
pub use iface::Iface;
pub(crate) use packet_slice::PacketSlice;
pub use phy::{EtherIface, IpIface};
pub(crate) use poll_iface::{PollKey, PollableIfaceMut};
pub use port::BindPortConfig;
pub use sched::ScheduleNextPoll;
