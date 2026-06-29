// SPDX-License-Identifier: MPL-2.0

//! Handle address-related requests.

use alloc::borrow::ToOwned;
use core::num::NonZeroU32;

use super::util::finish_response;
use crate::{
    net::{
        iface::{Iface, iter_all_ifaces},
        socket::netlink::{
            message::{CMsgSegHdr, CSegmentType, ErrorSegment, GetRequestFlags, SegHdrCommonFlags},
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
        .filter_map(|iface| iface_to_new_addr(request_segment.header(), iface))
        .map(RtnlSegment::NewAddr)
        .collect();

    finish_response(request_segment.header(), dump_all, &mut response_segments);

    Ok(response_segments)
}

fn iface_to_new_addr(request_header: &CMsgSegHdr, iface: &Arc<Iface>) -> Option<AddrSegment> {
    let ipv4_addr = iface.ipv4_addr()?;

    let header = CMsgSegHdr {
        len: 0,
        type_: CSegmentType::NEWADDR as _,
        flags: SegHdrCommonFlags::empty().bits(),
        seq: request_header.seq,
        pid: request_header.pid,
    };

    let addr_message = AddrSegmentBody {
        family: CSocketAddrFamily::AF_INET as _,
        prefix_len: iface.prefix_len().unwrap(),
        flags: AddrMessageFlags::PERMANENT,
        scope: RtScope::HOST,
        index: NonZeroU32::new(iface.index()),
    };

    let attrs = vec![
        AddrAttr::Address(ipv4_addr.octets()),
        AddrAttr::Label(iface.name().to_owned()),
        AddrAttr::Local(ipv4_addr.octets()),
    ];

    Some(AddrSegment::new(header, addr_message, attrs))
}

pub(super) fn do_new_addr(request_segment: &AddrSegment) -> Result<Vec<RtnlSegment>> {
    use aster_bigtcp::wire::{Ipv4Address, Ipv4Cidr};

    let body = request_segment.body();

    // Only AF_INET supported for now
    if body.family != CSocketAddrFamily::AF_INET as i32 {
        return_errno_with_message!(Errno::EAFNOSUPPORT, "only AF_INET is supported");
    }

    let prefix_len = body.prefix_len;

    // Extract IFA_ADDRESS or IFA_LOCAL from attributes
    let mut ipv4_addr: Option<[u8; 4]> = None;
    for attr in request_segment.attrs() {
        match attr {
            AddrAttr::Address(octets) => { ipv4_addr = Some(*octets); }
            AddrAttr::Local(octets)   => { if ipv4_addr.is_none() { ipv4_addr = Some(*octets); } }
            _ => {}
        }
    }

    let octets = ipv4_addr.ok_or_else(|| {
        Error::with_message(Errno::EINVAL, "no address provided in RTM_NEWADDR")
    })?;

    let smoltcp_addr = Ipv4Address::new(octets[0], octets[1], octets[2], octets[3]);
    let cidr = Ipv4Cidr::new(smoltcp_addr, prefix_len);

    // Find the target interface by index
    let iface_index = body.index.map(|n| n.get()).unwrap_or(0);
    let iface = iter_all_ifaces()
        .find(|iface| iface.index() == iface_index)
        .ok_or_else(|| Error::with_message(Errno::ENODEV, "interface not found"))?;

    iface.set_ipv4_cidr(cidr);

    // Send ACK (NLMSG_ERROR with err=0) as Linux does for successful write operations
    let ack = ErrorSegment::new_from_request(request_segment.header(), None);
    Ok(vec![RtnlSegment::Error(ack)])
}

