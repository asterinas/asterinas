// SPDX-License-Identifier: MPL-2.0

//! The kernel module defines the kernel socket,
//! which is responsible to handle the request from user space.

use core::marker::PhantomData;

use util::{add_multi_flag, append_done_segment};

use super::message::{NlMsg, NlSegment};
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

    pub fn request<F: FnMut(NlMsg)>(&self, request: &NlMsg, mut consume_response: F) -> Result<()> {
        debug!("netlink route request: {:?}", request);

        for segment in request.segments() {
            let request_header = segment.header();
            let response_segments = match CSegmentType::try_from(request_header.type_)? {
                CSegmentType::GETLINK => link::do_get_link(&segment),
                CSegmentType::GETADDR => addr::do_get_addr(&segment),
                _ => todo!(),
            };

            trace!("response segments: {:?}", response_segments);

            let response = match response_segments {
                Err(error) => {
                    let err_segment = ErrorSegment::new(request_header, Some(error));
                    NlMsg::new(vec![NlSegment::Error(err_segment)])
                }
                Ok(mut segments) => {
                    append_done_segment(request_header, &mut segments);
                    add_multi_flag(&mut segments);
                    NlMsg::new(segments)
                }
            };

            debug!("response: {:?}", response);

            consume_response(response);
        }

        Ok(())
    }
}

/// FIXME: NETLINK_ROUTE_KERNEL should be a per net namespace socket
static NETLINK_ROUTE_KERNEL: NetlinkRouteKernelSocket = NetlinkRouteKernelSocket::new();

pub fn get_netlink_route_kernel() -> &'static NetlinkRouteKernelSocket {
    &NETLINK_ROUTE_KERNEL
}
