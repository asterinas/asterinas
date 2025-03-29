// SPDX-License-Identifier: MPL-2.0

//! Deal with link-related requests.

use core::num::{NonZero, NonZeroU32};

use aster_bigtcp::iface::InterfaceType;

use crate::{
    net::{
        iface::{Iface, IFACES},
        socket::netlink::route::message::{
            attr::link::NlLinkAttr, CMessageSegmentHeader, CSegmentType, GetRequestFlags,
            LinkSegment, LinkSegmentBody, NlSegment, SegmentHeaderCommonFlags,
        },
    },
    prelude::*,
    util::net::CSocketAddrFamily,
};

pub fn do_get_link(segment: &NlSegment) -> Result<Vec<NlSegment>> {
    let NlSegment::Link(request_segment) = segment else {
        unreachable!("[Internal Error] Getlink request should only have link segment");
    };

    if !request_segment.body().flags.is_empty()
        || request_segment.body().type_ != InterfaceType::NETROM
    {
        return_errno_with_message!(Errno::EINVAL, "The request flags and type should be empty");
    }

    let flags = GetRequestFlags::from_bits_truncate(request_segment.header().flags);

    let ifaces = IFACES.get().unwrap();
    let response_segments = ifaces
        .iter()
        .filter(|iface| {
            if flags.contains(GetRequestFlags::DUMP) {
                return true;
            }

            if let Some(required_index) = request_segment.body().index.map(NonZeroU32::get) {
                if required_index != iface.index() {
                    return false;
                }
            }

            if let Some(required_name) = request_segment.attrs().iter().find_map(|attr| {
                if let NlLinkAttr::Name(name) = attr {
                    Some(name)
                } else {
                    None
                }
            }) {
                let required_name = required_name.to_str().unwrap();
                if required_name != iface.name() {
                    return false;
                }
            }

            true
        })
        .map(|iface| iface_to_new_link(request_segment.header(), iface))
        .map(|link_segment| NlSegment::Link(link_segment))
        .collect::<Vec<_>>();

    if response_segments.is_empty() {
        return_errno_with_message!(Errno::ENODEV, "no link is found");
    }

    Ok(response_segments)
}

fn iface_to_new_link(request_header: &CMessageSegmentHeader, iface: &Arc<Iface>) -> LinkSegment {
    let header = CMessageSegmentHeader {
        len: 0,
        type_: CSegmentType::NEWLINK as _,
        flags: SegmentHeaderCommonFlags::empty().bits(),
        seq: request_header.seq,
        pid: request_header.pid,
    };

    let link_message = LinkSegmentBody {
        family: CSocketAddrFamily::AF_UNSPEC,
        type_: iface.type_(),
        index: NonZero::new(iface.index()),
        flags: iface.flags(),
    };

    let attrs = vec![
        NlLinkAttr::Name(CString::new(iface.name()).unwrap()),
        NlLinkAttr::Mtu(iface.mtu() as u32),
    ];

    LinkSegment::new(header, link_message, attrs)
}
