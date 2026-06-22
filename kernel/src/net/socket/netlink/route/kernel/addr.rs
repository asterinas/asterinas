// SPDX-License-Identifier: MPL-2.0

//! Handle address-related requests.

use alloc::borrow::ToOwned;
use core::num::NonZeroU32;

use super::util::finish_response;
use crate::{
    net::{
        iface::{Iface, iter_all_ifaces},
        socket::netlink::{
            message::{CMsgSegHdr, CSegmentType, GetRequestFlags, SegHdrCommonFlags},
            route::message::{
                AddrAttr, AddrMessageFlags, AddrSegment, AddrSegmentBody, RtScope, RtnlSegment,
            },
        },
    },
    prelude::*,
    util::net::CSocketAddrFamily,
};

pub(super) fn do_get_addr(request_segment: &AddrSegment) -> Result<Vec<RtnlSegment>> {
    let dump_all = {
        let flags = GetRequestFlags::from_bits_truncate(request_segment.header().flags);
        flags.contains(GetRequestFlags::DUMP)
    };
    if !dump_all {
        return_errno_with_message!(Errno::EOPNOTSUPP, "GETADDR only supports dump requests");
    }

    let mut response_segments: Vec<RtnlSegment> = iter_all_ifaces()
        // GETADDR only supports dump mode, so we're going to report all addresses.
        .flat_map(|iface| iface_to_new_addr(request_segment.header(), iface))
        .map(RtnlSegment::NewAddr)
        .collect();

    finish_response(request_segment.header(), dump_all, &mut response_segments);

    Ok(response_segments)
}

fn iface_to_new_addr(request_header: &CMsgSegHdr, iface: &Arc<Iface>) -> Vec<AddrSegment> {
    let mut segments = Vec::new();

    if let Some(ipv4_addr) = iface.ipv4_addr() {
        let prefix_len = iface.ipv4_prefix_len().unwrap();
        segments.push(new_addr_segment(
            request_header,
            iface,
            CSocketAddrFamily::AF_INET,
            prefix_len,
            ipv4_addr.octets().to_vec(),
        ));
    }

    if let Some(ipv6_addr) = iface.ipv6_addr() {
        let prefix_len = iface.ipv6_prefix_len().unwrap();
        segments.push(new_addr_segment(
            request_header,
            iface,
            CSocketAddrFamily::AF_INET6,
            prefix_len,
            ipv6_addr.octets().to_vec(),
        ));
    }

    segments
}

fn new_addr_segment(
    request_header: &CMsgSegHdr,
    iface: &Arc<Iface>,
    family: CSocketAddrFamily,
    prefix_len: u8,
    address: Vec<u8>,
) -> AddrSegment {
    let header = CMsgSegHdr {
        len: 0,
        type_: CSegmentType::NEWADDR as _,
        flags: SegHdrCommonFlags::empty().bits(),
        seq: request_header.seq,
        pid: request_header.pid,
    };

    let addr_message = AddrSegmentBody {
        family: family as _,
        prefix_len,
        flags: AddrMessageFlags::PERMANENT,
        scope: RtScope::HOST,
        index: NonZeroU32::new(iface.index()),
    };

    // Linux reports `IFA_ADDRESS`, `IFA_LOCAL` and `IFA_LABEL` for IPv4 addresses
    // (`inet_fill_ifaddr`) but only `IFA_ADDRESS` for IPv6 addresses
    // (`inet6_fill_ifaddr`).
    let attrs = match family {
        CSocketAddrFamily::AF_INET6 => vec![AddrAttr::Address(address)],
        _ => vec![
            AddrAttr::Address(address.clone()),
            AddrAttr::Label(iface.name().to_owned()),
            AddrAttr::Local(address),
        ],
    };

    AddrSegment::new(header, addr_message, attrs)
}
