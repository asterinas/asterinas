// SPDX-License-Identifier: MPL-2.0

mod common;
#[expect(clippy::module_inception)]
mod iface;
mod phy;
mod poll;
mod poll_iface;
mod port;
mod sched;
mod time;

pub use common::{BoundPort, InterfaceFlags, InterfaceType};
pub use iface::Iface;
pub use phy::{EtherIface, IpIface};
pub(crate) use poll_iface::{PollKey, PollableIfaceMut};
pub use port::BindPortConfig;
pub use sched::ScheduleNextPoll;
