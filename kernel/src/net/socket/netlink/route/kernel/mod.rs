// SPDX-License-Identifier: MPL-2.0

//! The kernel module defines the kernel socket,
//! which is responsible to handle the request from user space.

use core::marker::PhantomData;

use super::message::NlMsg;
use crate::{net::socket::netlink::route::message::CSegmentType, prelude::*};

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
            let header = segment.header();
            let response = match CSegmentType::try_from(header.type_)? {
                CSegmentType::GETLINK => link::do_get_link(segment.as_ref()),
                CSegmentType::GETADDR => addr::do_get_addr(segment.as_ref()),
                _ => todo!(),
            };

            debug!("netlink route response: {:?}", response);
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
