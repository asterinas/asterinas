// SPDX-License-Identifier: MPL-2.0

//! This module defines the kernel socket,
//! which is responsible for handling requests from user space.

use core::marker::PhantomData;

use util::{add_multi_flag, append_done_segment};

use super::message::{Message, MsgSegment};
use crate::{
    net::socket::netlink::route::message::{CSegmentType, ErrorSegment},
    prelude::*,
};

mod addr;
mod link;
mod util;

pub struct NetlinkRouteKernelSocket {
    _private: PhantomData<()>,
}

impl NetlinkRouteKernelSocket {
    const fn new() -> Self {
        Self {
            _private: PhantomData,
        }
    }

    pub fn request<F: FnMut(Message)>(&self, request: &Message, mut consume_response: F) {
        debug!("netlink route request: {:?}", request);

        for segment in request.segments() {
            let request_header = segment.header();
            let Ok(segment_type) = CSegmentType::try_from(request_header.type_) else {
                // FIXME: The error is currently silently ignored.
                // Should we respond with an error segment for this and subsequent ignored errors?
                warn!("Invalid segment type");
                return;
            };
            let response_segments = match segment {
                MsgSegment::Link(request_segment) => {
                    match segment_type {
                        CSegmentType::GETLINK => link::do_get_link(request_segment),
                        _ => {
                            // FIXME: The error is currently silently ignored.
                            warn!("unsupported link request type");
                            return;
                        }
                    }
                }
                MsgSegment::Addr(request_segment) => {
                    match segment_type {
                        CSegmentType::GETADDR => addr::do_get_addr(request_segment),
                        _ => {
                            // FIXME: The error is currently silently ignored.
                            warn!("unsupported address request type");
                            return;
                        }
                    }
                }
                _ => {
                    // FIXME: The error is currently silently ignored.
                    warn!("unsupported request type");
                    return;
                }
            };

            let response = match response_segments {
                Err(error) => {
                    let err_segment = ErrorSegment::new_from_request(request_header, Some(error));
                    Message::new(vec![MsgSegment::Error(err_segment)])
                }
                Ok(mut segments) => {
                    append_done_segment(request_header, &mut segments);
                    add_multi_flag(&mut segments);
                    Message::new(segments)
                }
            };

            debug!("netlink route response: {:?}", response);

            consume_response(response);
        }
    }
}

/// FIXME: NETLINK_ROUTE_KERNEL should be a per-network namespace socket
static NETLINK_ROUTE_KERNEL: NetlinkRouteKernelSocket = NetlinkRouteKernelSocket::new();

pub fn get_netlink_route_kernel() -> &'static NetlinkRouteKernelSocket {
    &NETLINK_ROUTE_KERNEL
}
