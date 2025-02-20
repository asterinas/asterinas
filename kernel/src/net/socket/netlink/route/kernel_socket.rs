// SPDX-License-Identifier: MPL-2.0

use core::{
    marker::PhantomData,
    num::{NonZero, NonZeroU32},
};

use ostd::early_println;

use super::message::{
    AckMessage, AnyRequestMessage, AnyResponseMessage, AttrOps, IfName, LinkSegment, NlMsg,
    NlMsgSegment, ReadAttrFromUser, ReadNlMsgSegmentFromUser, ResponseContent, TxqLen,
};
use crate::{
    net::{
        iface::{ConfigurableIface, IFACES},
        socket::netlink::{
            message::{CNetlinkMessageHeader, GetRequestFlags, NetlinkMessageCommonFlags},
            route::message::{
                AddrMessage, AddrMessageFlags, CMessageType, GetRequest, GetResponse, IfaAddress,
                IfaLable, IfaLocal, LinkMessage, RtScope,
            },
        },
    },
    prelude::*,
    util::net::CSocketAddrFamily,
};

pub struct NetlinkRouteKernelSocket {
    _private: PhantomData<()>,
}

impl NetlinkRouteKernelSocket {
    const fn new() -> Self {
        Self {
            _private: PhantomData,
        }
    }

    pub fn request<F: Fn(Box<dyn AnyResponseMessage>)>(
        &self,
        message: &dyn AnyRequestMessage,
        consume_response: F,
    ) -> Result<()> {
        let response = if let Some(get_link_request) =
            message.as_any().downcast_ref::<GetRequest<LinkMessage>>()
        {
            do_get_link(get_link_request)
        } else if let Some(get_addr_request) =
            message.as_any().downcast_ref::<GetRequest<AddrMessage>>()
        {
            do_get_addr(get_addr_request)
        } else {
            todo!()
        };

        consume_response(Box::new(response));
        return Ok(());
    }

    pub fn request_new<F: Fn(Box<dyn AnyResponseMessage>)>(
        &self,
        message: &NlMsg,
        consume_response: F,
    ) -> Result<()> {
        for segment in &message.segments {
            let header = segment.header();
            match CMessageType::try_from(header.type_)? {
                CMessageType::GETLINK => do_get_link_new(segment.as_ref()),
                _ => todo!(),
            }
        }

        todo!()
    }
}

/// FIXME: NETLINK_ROUTE_KERNEL should be a per net namespace socket
static NETLINK_ROUTE_KERNEL: NetlinkRouteKernelSocket = NetlinkRouteKernelSocket::new();

pub fn get_netlink_route_kernel() -> &'static NetlinkRouteKernelSocket {
    &NETLINK_ROUTE_KERNEL
}

fn do_get_link_new(segment: &dyn NlMsgSegment) -> NlMsg {
    let segment = segment.as_any().downcast_ref::<LinkSegment>().unwrap();
    let flags = GetRequestFlags::from_bits_truncate(segment.header().flags);

    let ifaces = IFACES.get().unwrap();
    let links = ifaces
        .iter()
        .filter(|iface| {
            if flags.contains(GetRequestFlags::DUMP) {
                return true;
            }

            if let Some(required_index) = segment.body().index.map(NonZeroU32::get) {
                if required_index != *iface.index() {
                    return false;
                }
            }

            if let Some(if_name) = segment
                .attrs()
                .iter()
                .filter_map(|attr| attr.as_any().downcast_ref::<IfName>())
                .nth(0)
            {
                let required_name = if_name.value.to_str().unwrap();
                if required_name != iface.name().as_str() {
                    return false;
                }
            }

            true
        })
        .map(|iface| iface_to_new_link(segment.header(), iface));
    todo!()
}

fn iface_to_new_link(
    request_header: &CNetlinkMessageHeader,
    iface: &ConfigurableIface,
) -> LinkSegment {
    let link_message = LinkMessage {
        family: *iface.family(),
        type_: *iface.type_(),
        index: NonZero::new(*iface.index()),
        flags: *iface.flags(),
    };

    let attrs = vec![
        Box::new(IfName::new(CString::new(iface.name().as_str()).unwrap())) as Box<dyn AttrOps>,
        Box::new(TxqLen::new((*iface.txqlen()) as u32)),
    ];

    let header = CNetlinkMessageHeader {
        len: 0,
        type_: CMessageType::NEWLINK as _,
        flags: NetlinkMessageCommonFlags::empty().bits(),
        seq: request_header.seq,
        pid: request_header.pid,
    };

    LinkSegment::new(header, link_message, attrs)
}

fn do_get_link(request: &GetRequest<LinkMessage>) -> GetResponse {
    let ifaces = IFACES.get().unwrap();

    let links: Vec<_> = ifaces
        .iter()
        .filter(|iface| {
            if request.flags().contains(GetRequestFlags::DUMP) {
                return true;
            }

            if let Some(required_index) = request.message().index.map(NonZeroU32::get) {
                if required_index != *iface.index() {
                    return false;
                }
            }

            // if let Some(if_name) = request
            //     .message()
            //     .attrs
            //     .iter()
            //     .filter_map(|attr| attr.as_any().downcast_ref::<IfName>())
            //     .nth(0)
            // {
            //     let required_name = if_name.value.to_str().unwrap();
            //     if required_name != iface.name().as_str() {
            //         return false;
            //     }
            // }

            true
        })
        .map(|iface| LinkMessage {
            family: *iface.family(),
            type_: *iface.type_(),
            index: NonZero::new(*iface.index()),
            flags: *iface.flags(),
            // attrs: vec![
            //     Box::new(IfName::new(CString::new(iface.name().as_str()).unwrap())),
            //     Box::new(TxqLen::new((*iface.txqlen()) as u32)),
            // ],
        })
        .collect();

    let ack_message = if links.len() == 0 {
        Some(AckMessage::new_error(Errno::ENODEV, *request.header()))
    } else if links.len() > 1 {
        Some(AckMessage::new_done(0, request.header()))
    } else {
        None
    };

    GetResponse::new(request.header(), ResponseContent::Link(links, ack_message))
}

fn do_get_addr(request: &GetRequest<AddrMessage>) -> GetResponse {
    early_println!("request = {:?}", request);

    let ifaces = IFACES.get().unwrap();

    let addrs: Vec<_> = ifaces
        .iter()
        .filter(|iface| {
            if iface.iface().ipv4_addr().is_none() {
                return false;
            }

            if request.flags().contains(GetRequestFlags::DUMP) {
                return true;
            }

            if let Some(index) = request.message().index {
                if *iface.index() != index.get() {
                    return false;
                }
            }

            true
        })
        .map(|iface| {
            let ipv4_addr = iface.iface().ipv4_addr().unwrap();

            AddrMessage {
                family: CSocketAddrFamily::AF_INET,
                prefix_len: iface.iface().prefix_len().unwrap(),
                flags: AddrMessageFlags::PERMANENT,
                scope: RtScope::HOST,
                index: NonZeroU32::new(*iface.index()),
                // attrs: vec![
                //     Box::new(IfaAddress::new(u32::from_ne_bytes(ipv4_addr.octets()))),
                //     Box::new(IfaLable::new(CString::new(iface.name().as_str()).unwrap())),
                //     Box::new(IfaLocal::new(u32::from_ne_bytes(ipv4_addr.octets()))),
                // ],
            }
        })
        .collect();

    let ack_message = if addrs.len() == 0 {
        Some(AckMessage::new_error(Errno::ENODEV, *request.header()))
    } else if addrs.len() > 1 {
        Some(AckMessage::new_done(0, request.header()))
    } else {
        None
    };

    GetResponse::new(request.header(), ResponseContent::Addr(addrs, ack_message))
}
