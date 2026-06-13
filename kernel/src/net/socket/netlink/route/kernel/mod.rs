// SPDX-License-Identifier: MPL-2.0

//! This module defines the kernel socket,
//! which is responsible for handling requests from user space.

use core::marker::PhantomData;

use ostd::task::Task;

use super::message::{RtnlMessage, RtnlSegment};
use crate::{
    net::socket::netlink::{
        addr::PortNum,
        message::{ErrorSegment, ProtocolSegment},
        table::{NetlinkRouteProtocol, SupportedNetlinkProtocol},
    },
    prelude::*,
};

pub(in crate::net) mod addr;
pub(in crate::net) mod link;
pub(in crate::net) mod util;

/// The kernel-side netlink route socket.
///
/// Each network namespace owns one instance of this socket, which handles
/// netlink route requests (e.g. link/address queries) originating from
/// user space within that namespace.
///
/// This is a zero-sized type (contains only `PhantomData`), so it is
/// `Copy` — we can return it by value from a per-namespace lookup
/// without needing `unsafe` lifetime extension.
#[derive(Copy, Clone)]
pub(in crate::net) struct NetlinkRouteKernelSocket {
    _private: PhantomData<()>,
}

impl NetlinkRouteKernelSocket {
    pub(in crate::net) const fn new() -> Self {
        Self {
            _private: PhantomData,
        }
    }

    pub(super) fn handle_request(&self, request: &RtnlSegment, dst_port: PortNum) {
        debug!("netlink route request: {:?}", request);

        let request_header = request.header();

        let response_segments = match request {
            RtnlSegment::GetLink(request_segment) => link::do_get_link(request_segment),
            RtnlSegment::GetAddr(request_segment) => addr::do_get_addr(request_segment),
            _ => Err(Error::with_message(
                Errno::EOPNOTSUPP,
                "the netlink route request is not supported",
            )),
        };

        let response = match response_segments {
            Ok(segments) => RtnlMessage::new(segments),
            Err(error) => {
                // TODO: Deal with the `NetlinkMessageCommonFlags::ACK` flag.
                // Should we return `ErrorSegment` if ACK flag does not exist?
                // Reference: <https://docs.kernel.org/userspace-api/netlink/intro.html#netlink-message-types>.
                let err_segment = ErrorSegment::new_from_request(request_header, Some(error));
                self.report_error(err_segment, dst_port);
                return;
            }
        };

        debug!("netlink route response: {:?}", response);

        NetlinkRouteProtocol::unicast(dst_port, response).unwrap();
    }

    pub(super) fn report_error(&self, err_segment: ErrorSegment, dst_port: PortNum) {
        let response = RtnlMessage::new(vec![RtnlSegment::Error(err_segment)]);

        debug!("netlink route error: {:?}", response);

        NetlinkRouteProtocol::unicast(dst_port, response).unwrap();
    }
}

/// Returns the kernel-side netlink route socket for the current thread's
/// network namespace.
///
/// `NetlinkRouteKernelSocket` is a zero-sized `Copy` type, so we return it
/// by value from the per-namespace instance — no `unsafe` required.
pub(super) fn get_netlink_route_kernel() -> NetlinkRouteKernelSocket {
    let current_task = Task::current().unwrap();
    let thread_local = current_task.as_thread_local().unwrap();
    let ns_proxy_ref = thread_local.borrow_ns_proxy();
    let ns_proxy = ns_proxy_ref.unwrap();
    ns_proxy.net_ns().netlink_route_kernel()
}
