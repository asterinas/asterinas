// SPDX-License-Identifier: MPL-2.0

//! Deal with address-related requests.

use core::num::NonZeroU32;

use crate::{
    net::{
        iface::{Iface, IFACES},
        socket::netlink::route::message::{
            attr::addr::NlAddrAttr, AddrMessageFlags, AddrSegment, AddrSegmentBody,
            CMessageSegmentHeader, CSegmentType, GetRequestFlags, NlSegment, RtScope,
            SegmentHeaderCommonFlags,
        },
    },
    prelude::*,
    util::net::CSocketAddrFamily,
};

pub fn do_get_addr(segment: &NlSegment) -> Result<Vec<NlSegment>> {
    let NlSegment::Addr(request_segment) = segment else {
        unreachable!("[Internal Error] Getaddr request should only have addr segment");
    };
    let flags = GetRequestFlags::from_bits_truncate(request_segment.header().flags);

    let ifaces = IFACES.get().unwrap();
    let response_segments = ifaces
        .iter()
        .filter(|iface| {
            // Filter only requested addresses

            if iface.ipv4_addr().is_none() {
                return false;
            }

            if flags.contains(GetRequestFlags::DUMP) {
                return true;
            }

            if let Some(index) = request_segment.body().index.map(NonZeroU32::get) {
                if iface.index() != index {
                    return false;
                }
            }

            // Asterinas's devices currently only support IPv4 addresses.
            // Therefore, requests for other address types can be safely ignored.
            // FIXME: Update the logic once Asterinas supports additional address families.
            match request_segment.body().family {
                CSocketAddrFamily::AF_UNSPEC | CSocketAddrFamily::AF_INET => true,
                _ => false,
            }
        })
        .map(|iface| iface_to_new_addr(request_segment.header(), iface))
        .map(|addr_segment| NlSegment::Addr(addr_segment))
        .collect::<Vec<_>>();

    if response_segments.is_empty() {
        // FIXME: This error is just from getlink, we need to further check
        // whether Linux uses the error number.
        return_errno_with_message!(Errno::ENODEV, "no address is found");
    }

    Ok(response_segments)
}

fn iface_to_new_addr(request_header: &CMessageSegmentHeader, iface: &Arc<Iface>) -> AddrSegment {
    let ipv4_addr = iface.ipv4_addr().unwrap();

    let header = CMessageSegmentHeader {
        len: 0,
        type_: CSegmentType::NEWADDR as _,
        flags: SegmentHeaderCommonFlags::empty().bits(),
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
        NlAddrAttr::Address(ipv4_addr.octets()),
        NlAddrAttr::Label(CString::new(iface.name()).unwrap()),
        NlAddrAttr::Local(ipv4_addr.octets()),
    ];

    AddrSegment::new(header, addr_message, attrs)
}
