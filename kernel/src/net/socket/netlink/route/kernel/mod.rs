// SPDX-License-Identifier: MPL-2.0

//! This module defines the kernel socket,
//! which is responsible for handling requests from user space.

use core::marker::PhantomData;

use super::message::{RtnlMessage, RtnlSegment};
use crate::{
    net::socket::netlink::{
        addr::PortNum,
        message::{CSegmentType, ErrorSegment, ProtocolSegment},
        table::{NetlinkRouteProtocol, SupportedNetlinkProtocol},
    },
    prelude::*,
};

mod addr;
mod link;
mod util;

pub(super) struct NetlinkRouteKernelSocket {
    _private: PhantomData<()>,
}

impl NetlinkRouteKernelSocket {
    const fn new() -> Self {
        Self {
            _private: PhantomData,
        }
    }

    pub(super) fn request(&self, request: &RtnlMessage, dst_port: PortNum) {
        debug!("netlink route request: {:?}", request);

        for segment in request.segments() {
            let request_header = segment.header();

            let segment_type = CSegmentType::try_from(request_header.type_).unwrap();

            let response_segments = match segment {
                RtnlSegment::GetLink(request_segment) => link::do_get_link(request_segment),
                RtnlSegment::GetAddr(request_segment) => addr::do_get_addr(request_segment),
                _ => {
                    // FIXME: The error is currently silently ignored.
                    warn!("unsupported request type: {:?}", segment_type);
                    return;
                }
            };

            let response = match response_segments {
                Ok(segments) => RtnlMessage::new(segments),
                Err(error) => {
                    // TODO: Deal with the `NetlinkMessageCommonFlags::ACK` flag.
                    // Should we return `ErrorSegment` if ACK flag does not exist?
                    // Reference: <https://docs.kernel.org/userspace-api/netlink/intro.html#netlink-message-types>.
                    let err_segment = ErrorSegment::new_from_request(request_header, Some(error));
                    RtnlMessage::new(vec![RtnlSegment::Error(err_segment)])
                }
            };

            debug!("netlink route response: {:?}", response);

            NetlinkRouteProtocol::unicast(dst_port, response).unwrap();
        }
    }
}

/// FIXME: NETLINK_ROUTE_KERNEL should be a per-network namespace socket
static NETLINK_ROUTE_KERNEL: NetlinkRouteKernelSocket = NetlinkRouteKernelSocket::new();

pub(super) fn get_netlink_route_kernel() -> &'static NetlinkRouteKernelSocket {
    &NETLINK_ROUTE_KERNEL
}
