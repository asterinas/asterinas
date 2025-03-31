// SPDX-License-Identifier: MPL-2.0

//! Handle address-related requests.

use core::num::NonZeroU32;

use crate::{
    net::{
        iface::{Iface, IFACES},
        socket::netlink::route::message::{
            attr::addr::AddrAttr, AddrMessageFlags, AddrSegment, AddrSegmentBody,
            CMessageSegmentHeader, CSegmentType, GetRequestFlags, MsgSegment, RtScope,
            SegmentHeaderCommonFlags,
        },
    },
    prelude::*,
    util::net::CSocketAddrFamily,
};

pub fn do_get_addr(request_segment: &AddrSegment) -> Result<Vec<MsgSegment>> {
    let dump_all = {
        let flags = GetRequestFlags::from_bits_truncate(request_segment.header().flags);
        flags.contains(GetRequestFlags::DUMP)
    };

    let required_index = request_segment.body().index.map(NonZeroU32::get);

    if dump_all && required_index.is_some() {
        return_errno_with_message!(
            Errno::EINVAL,
            "filtering by device index is not supported for address dumps"
        );
    }

    let ifaces = IFACES.get().unwrap();
    let response_segments: Vec<MsgSegment> = ifaces
        .iter()
        .filter(|iface| {
            // Filter to include only requested addresses.

            // Since `iface_to_new_addr` (called in the next `map` closure) will unwrap `iface.ipv4_addr`,
            // we need to filter out all interfaces without an `ipv4_addr`,
            // even if `dump_all` is true.
            if iface.ipv4_addr().is_none() {
                return false;
            }

            if dump_all {
                return true;
            }

            // Asterinas devices currently only support IPv4 addresses.
            // Therefore, requests for other address types can be safely ignored.
            // FIXME: Update the logic once Asterinas supports additional address families.
            match request_segment.body().family {
                CSocketAddrFamily::AF_UNSPEC | CSocketAddrFamily::AF_INET => {}
                _ => return false,
            }

            if let Some(required_index) = required_index {
                return iface.index() == required_index;
            }

            true
        })
        .map(|iface| iface_to_new_addr(request_segment.header(), iface))
        .map(MsgSegment::Addr)
        .collect();

    if response_segments.is_empty() {
        // FIXME: This error is from getlink, we need to further verify
        // whether Linux uses the error number.
        return_errno_with_message!(Errno::ENODEV, "no addresses found");
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
        AddrAttr::Address(ipv4_addr.octets()),
        AddrAttr::Label(CString::new(iface.name()).unwrap()),
        AddrAttr::Local(ipv4_addr.octets()),
    ];

    AddrSegment::new(header, addr_message, attrs)
}
