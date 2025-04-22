// SPDX-License-Identifier: MPL-2.0

//! Handle link-related requests.

use core::num::NonZero;

use aster_bigtcp::iface::InterfaceType;

use super::util::finish_response;
use crate::{
    net::{
        iface::{iter_all_ifaces, Iface},
        socket::netlink::{
            message::{CMsgSegHdr, CSegmentType, GetRequestFlags, SegHdrCommonFlags},
            route::message::{LinkAttr, LinkSegment, LinkSegmentBody, RtnlSegment},
        },
    },
    prelude::*,
    util::net::CSocketAddrFamily,
};

pub(super) fn do_get_link(request_segment: &LinkSegment) -> Result<Vec<RtnlSegment>> {
    let filter_by = FilterBy::from_request(request_segment)?;

    let mut response_segments: Vec<RtnlSegment> = iter_all_ifaces()
        // Filter to include only requested links.
        .filter(|iface| match &filter_by {
            FilterBy::Index(index) => *index == iface.index(),
            FilterBy::Name(name) => *name == iface.name(),
            FilterBy::Dump => true,
        })
        .map(|iface| iface_to_new_link(request_segment.header(), iface))
        .map(RtnlSegment::NewLink)
        .collect();

    let dump_all = matches!(filter_by, FilterBy::Dump);

    if !dump_all && response_segments.is_empty() {
        return_errno_with_message!(Errno::ENODEV, "no link found");
    }

    finish_response(request_segment.header(), dump_all, &mut response_segments);

    Ok(response_segments)
}

enum FilterBy<'a> {
    Index(u32),
    Name(&'a str),
    Dump,
}

impl<'a> FilterBy<'a> {
    fn from_request(request_segment: &'a LinkSegment) -> Result<Self> {
        let dump_all = {
            let flags = GetRequestFlags::from_bits_truncate(request_segment.header().flags);
            flags.contains(GetRequestFlags::DUMP)
        };
        if dump_all {
            validate_dumplink_request(request_segment.body())?;
            return Ok(Self::Dump);
        }

        validate_getlink_request(request_segment.body())?;

        // `index` takes precedence over `required_name`.

        if let Some(required_index) = request_segment.body().index {
            return Ok(Self::Index(required_index.get()));
        }

        let required_name = request_segment.attrs().iter().find_map(|attr| {
            if let LinkAttr::Name(name) = attr {
                Some(name.to_str().unwrap())
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

// The below functions starting with `validate_` should only be enabled in strict mode.
// Reference: <https://docs.kernel.org/userspace-api/netlink/intro.html#strict-checking>.

fn validate_getlink_request(body: &LinkSegmentBody) -> Result<()> {
    // FIXME: The Linux implementation also checks the `padding` and `change` fields,
    // but these fields are lost during the conversion of a `CIfInfoMsg` to `LinkSegmentBody`.
    // We should consider including the `change` field in `LinkSegmentBody`.
    // Reference: <https://elixir.bootlin.com/linux/v6.13/source/net/core/rtnetlink.c#L4043>.
    if !body.flags.is_empty() || body.type_ != InterfaceType::NETROM {
        return_errno_with_message!(Errno::EINVAL, "the flags or the type is not valid");
    }

    Ok(())
}

fn validate_dumplink_request(body: &LinkSegmentBody) -> Result<()> {
    // FIXME: The Linux implementation also checks the `padding` and `change` fields.
    // Reference: <https://elixir.bootlin.com/linux/v6.13/source/net/core/rtnetlink.c#L2378>.
    if !body.flags.is_empty() || body.type_ != InterfaceType::NETROM {
        return_errno_with_message!(Errno::EINVAL, "the flags or the type is not valid");
    }

    // The check is from <https://elixir.bootlin.com/linux/v6.13/source/net/core/rtnetlink.c#L2383>.
    if body.index.is_some() {
        return_errno_with_message!(
            Errno::EINVAL,
            "filtering by interface index is not valid for link dumps"
        );
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
