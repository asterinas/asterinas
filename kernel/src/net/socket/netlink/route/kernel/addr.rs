// SPDX-License-Identifier: MPL-2.0

//! Handle address-related requests.

use core::num::NonZeroU32;

use super::util::finish_response;
use crate::{
    net::{
        iface::{Iface, IFACES},
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

pub fn do_get_addr(request_segment: &AddrSegment) -> Result<Vec<RtnlSegment>> {
    let dump_all = {
        let flags = GetRequestFlags::from_bits_truncate(request_segment.header().flags);
        flags.contains(GetRequestFlags::DUMP)
    };

    if !dump_all {
        return_errno_with_message!(Errno::EOPNOTSUPP, "GETADDR only supports dump requests");
    }

    let ifaces = IFACES.get().unwrap();
    let mut response_segments: Vec<RtnlSegment> = ifaces
        .iter()
        .filter(|iface| {
            // Filter to include only requested addresses.

            // Since `iface_to_new_addr` (called in the next `map` closure) will unwrap `iface.ipv4_addr`,
            // we need to filter out all interfaces without an `ipv4_addr`,
            // even if `dump_all` is true.
            if iface.ipv4_addr().is_none() {
                return false;
            }

            // GETADDR only supports dump mode, so all addresses should be returned.
            true
        })
        .map(|iface| iface_to_new_addr(request_segment.header(), iface))
        .map(RtnlSegment::NewAddr)
        .collect();

    if response_segments.is_empty() {
        // TDDO: Should we return an error if no addresses are found?
    }

    finish_response(request_segment.header(), dump_all, &mut response_segments);

    Ok(response_segments)
}

fn iface_to_new_addr(request_header: &CMsgSegHdr, iface: &Arc<Iface>) -> AddrSegment {
    let ipv4_addr = iface.ipv4_addr().unwrap();

    let header = CMsgSegHdr {
        len: 0,
        type_: CSegmentType::NEWADDR as _,
        flags: SegHdrCommonFlags::empty().bits(),
        seq: request_header.seq,
        pid: request_header.pid,
    };

    let addr_message = AddrSegmentBody {
        family: CSocketAddrFamily::AF_INET,
        prefix_len: iface.prefix_len().unwrap(),
        flags: AddrMessageFlags::PERMANENT,
        scope: RtScope::HOST,
        index: NonZeroU32::new(iface.index()),
    };

    let attrs = vec![
        AddrAttr::Address(ipv4_addr.octets()),
        AddrAttr::Label(CString::new(iface.name()).unwrap()),
        AddrAttr::Local(ipv4_addr.octets()),
    ];

    AddrSegment::new(header, addr_message, attrs)
}
