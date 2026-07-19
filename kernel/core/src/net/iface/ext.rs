// SPDX-License-Identifier: MPL-2.0

use super::sched::PollScheduler;
use crate::net::socket::ip::{DatagramObserver, StreamObserver};

pub struct BigtcpExt;

impl aster_bigtcp::ext::Ext for BigtcpExt {
    type ScheduleNextPoll = PollScheduler;

    type TcpEventObserver = StreamObserver;
    type UdpEventObserver = DatagramObserver;
}
