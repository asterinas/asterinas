// SPDX-License-Identifier: MPL-2.0

//! Handle link-related requests.

use core::num::{NonZero, NonZeroU32};

use aster_bigtcp::iface::InterfaceType;

use crate::{
    net::{
        iface::{Iface, IFACES},
        socket::netlink::route::message::{
            attr::link::LinkAttr, CMessageSegmentHeader, CSegmentType, GetRequestFlags,
            LinkSegment, LinkSegmentBody, MsgSegment, SegmentHeaderCommonFlags,
        },
    },
    prelude::*,
    util::net::CSocketAddrFamily,
};

pub fn do_get_link(request_segment: &LinkSegment) -> Result<Vec<MsgSegment>> {
    validate_getlink_request_body(request_segment.body())?;

    let dump_all = {
        let flags = GetRequestFlags::from_bits_truncate(request_segment.header().flags);
        flags.contains(GetRequestFlags::DUMP)
    };
    let required_index = request_segment.body().index.map(NonZeroU32::get);
    let required_name = request_segment.attrs().iter().find_map(|attr| {
        if let LinkAttr::Name(name) = attr {
            Some(name.to_str().unwrap())
        } else {
            None
        }
    });

    if dump_all && required_index.is_some() {
        return_errno_with_message!(
            Errno::EINVAL,
            "filtering by device index is not supported for link dumps"
        );
    }

    let ifaces = IFACES.get().unwrap();
    let response_segments: Vec<MsgSegment> = ifaces
        .iter()
        .filter(|iface| {
            // Filter to include only requested links.

            if dump_all {
                return true;
            }

            // `required_index` takes precedence over `required_name`.

            if let Some(required_index) = required_index {
                return required_index == iface.index();
            }

            if let Some(required_name) = required_name {
                return required_name == iface.name();
            }

            true
        })
        .map(|iface| iface_to_new_link(request_segment.header(), iface))
        .map(MsgSegment::Link)
        .collect();

    if response_segments.is_empty() {
        return_errno_with_message!(Errno::ENODEV, "no link found");
    }

    Ok(response_segments)
}

fn validate_getlink_request_body(body: &LinkSegmentBody) -> Result<()> {
    // FIXME: The Linux implementation also checks the `padding` and `change` fields,
    // but these fields are lost during the conversion of a `CIfInfoMsg` to `LinkSegmentBody`.
    // We should consider including the `change` field in `LinkSegmentBody`.
    // Reference: https://elixir.bootlin.com/linux/v6.13/source/net/core/rtnetlink.c#L4043
    if !body.flags.is_empty() || body.type_ != InterfaceType::NETROM {
        return_errno_with_message!(Errno::EINVAL, "The empty flags and type_ are invalid");
    }

    Ok(())
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
        LinkAttr::Name(CString::new(iface.name()).unwrap()),
        LinkAttr::Mtu(iface.mtu() as u32),
    ];

    LinkSegment::new(header, link_message, attrs)
}
