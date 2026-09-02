// SPDX-License-Identifier: MPL-2.0

//! Handle link-related requests.

use aster_bigtcp::wire::EthernetAddress;

use super::util::{self, ResponseMode};
use crate::{
    net::{
        iface::{DEFAULT_TX_QUEUE_LEN, Iface, iter_all_ifaces},
        socket::netlink::{
            message::{CMsgSegHdr, CSegmentType, GetRequestFlags, SegHdrCommonFlags},
            route::message::{CIfinfoMsg, LinkAttr, LinkAttrClass, LinkSegment, RtnlSegment},
        },
    },
    prelude::*,
    util::net::CSocketAddrFamily,
};

/// The unspecified link-layer address.
const UNSPECIFIED_LINK_ADDR: EthernetAddress = EthernetAddress([0; 6]);

pub(super) fn do_get_link(
    request_segment: &LinkSegment,
    strict_check: bool,
) -> Result<Vec<RtnlSegment>> {
    validate_getlink_request(request_segment, strict_check)?;

    let filter_by = FilterBy::from_request(request_segment)?;

    let mut response_segments: Vec<RtnlSegment> = iter_all_ifaces()
        // Filter to include only requested links.
        .filter(|iface| match &filter_by {
            FilterBy::Index(index) => *index == iface.index(),
            FilterBy::Name(name) => *name == iface.name().as_cstr(),
            FilterBy::Dump => true,
        })
        .map(|iface| iface_to_new_link(request_segment.header(), iface))
        .map(RtnlSegment::NewLink)
        .collect();

    let response_mode = match filter_by {
        FilterBy::Dump => ResponseMode::Dump,
        FilterBy::Index(_) | FilterBy::Name(_) => ResponseMode::Single,
    };

    if response_mode == ResponseMode::Single && response_segments.is_empty() {
        return_errno_with_message!(Errno::ENODEV, "no link found");
    }

    util::finish_response(
        request_segment.header(),
        response_mode,
        &mut response_segments,
    );

    Ok(response_segments)
}

enum FilterBy<'a> {
    Index(u32),
    Name(&'a CStr),
    Dump,
}

impl<'a> FilterBy<'a> {
    fn from_request(request_segment: &'a LinkSegment) -> Result<Self> {
        let dump_all = {
            let flags = GetRequestFlags::from_bits_truncate(request_segment.header().flags);
            flags.contains(GetRequestFlags::DUMP)
        };
        if dump_all {
            return Ok(Self::Dump);
        }

        // `index` takes precedence over `required_name`.

        let required_index = request_segment.body().index;
        if required_index != 0 {
            return Ok(Self::Index(required_index));
        }

        let required_name = request_segment.attrs().iter().find_map(|attr| {
            if let LinkAttr::Name(name) = attr {
                Some(name.as_cstr())
            } else {
                None
            }
        });
        if let Some(required_name) = required_name {
            return Ok(Self::Name(required_name));
        }

        return_errno_with_message!(
            Errno::EINVAL,
            "either interface name or index should be specified for non-dump mode"
        );
    }
}

fn validate_getlink_request(request_segment: &LinkSegment, strict_check: bool) -> Result<()> {
    if !strict_check {
        return Ok(());
    }

    let body = request_segment.body();
    // GETLINK reserves these fields.
    // Reference: <https://elixir.bootlin.com/linux/v7.1/source/net/core/rtnetlink.c#L4141>.
    if body._pad != 0 || body.type_ != 0 || body.flags != 0 || body.change != 0 {
        return_errno_with_message!(
            Errno::EINVAL,
            "the link request header contains nonzero padding, type, flags, or change"
        );
    }

    // Linux accepts different attributes for link dumps and single-link queries
    // when strict checking is enabled.
    let flags = GetRequestFlags::from_bits_truncate(request_segment.header().flags);
    let allowed = if flags.contains(GetRequestFlags::DUMP) {
        if body.index != 0 {
            return_errno_with_message!(
                Errno::EINVAL,
                "filtering by interface index is not supported for link dumps"
            );
        }
        // Reference: <https://elixir.bootlin.com/linux/v7.1/source/net/core/rtnetlink.c#L2495>.
        [
            LinkAttrClass::MASTER,
            LinkAttrClass::LINKINFO,
            LinkAttrClass::EXT_MASK,
            LinkAttrClass::IF_NETNSID,
        ]
        .as_slice()
    } else {
        // Reference: <https://elixir.bootlin.com/linux/v7.1/source/net/core/rtnetlink.c#L4152>.
        [
            LinkAttrClass::IFNAME,
            LinkAttrClass::EXT_MASK,
            LinkAttrClass::IF_NETNSID,
        ]
        .as_slice()
    };
    for attr in request_segment.attrs() {
        if !allowed.contains(&attr.class()) {
            return_errno_with_message!(
                Errno::EINVAL,
                "the attribute is not allowed in this link request"
            );
        }
    }
    Ok(())
}

fn iface_to_new_link(request_header: &CMsgSegHdr, iface: &Arc<Iface>) -> LinkSegment {
    let header = CMsgSegHdr {
        len: 0,
        type_: CSegmentType::NEWLINK as _,
        flags: SegHdrCommonFlags::empty().bits(),
        seq: request_header.seq,
        pid: request_header.pid,
    };

    let link_message = CIfinfoMsg {
        family: CSocketAddrFamily::AF_UNSPEC as u8,
        _pad: 0,
        type_: iface.type_() as u16,
        index: iface.index(),
        flags: iface.flags().bits(),
        change: 0,
    };

    // Linux may report dozens of attributes in a fixed order.
    // See the reference below for the complete attribute list and ordering.
    // TODO: Asterinas currently reports only a subset of these attributes.
    // Reference: <https://elixir.bootlin.com/linux/v7.1/source/net/core/rtnetlink.c#L2050>.
    let mut attrs = Vec::with_capacity(5);
    attrs.extend([
        LinkAttr::Name(*iface.name()),
        LinkAttr::TxqLen(DEFAULT_TX_QUEUE_LEN),
        LinkAttr::Mtu(iface.mtu() as u32),
    ]);

    let (link_addr, link_broadcast_addr) = match iface.ethernet_addr() {
        Some(ethernet_addr) => (ethernet_addr, EthernetAddress::BROADCAST),
        None => (UNSPECIFIED_LINK_ADDR, UNSPECIFIED_LINK_ADDR),
    };
    attrs.extend([
        LinkAttr::Address(link_addr),
        LinkAttr::Broadcast(link_broadcast_addr),
    ]);

    LinkSegment::new(header, link_message, attrs)
}
