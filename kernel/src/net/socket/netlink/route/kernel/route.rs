// SPDX-License-Identifier: MPL-2.0

//! Handles route-related requests.

use core::num::NonZeroU32;

use aster_bigtcp::wire::{IpAddress, Ipv4Address, Ipv6Address, Ipv6AddressExt};

use super::util;
use crate::{
    net::{
        route::{
            self, RouteEntry, RouteLookupKey, RouteProtocol, RouteScope, RouteTableId, RouteType,
        },
        socket::netlink::{
            message::{CMsgSegHdr, CSegmentType, GetRequestFlags, SegHdrCommonFlags},
            route::message::{RouteAttr, RouteFlags, RouteSegment, RouteSegmentBody, RtnlSegment},
        },
    },
    prelude::*,
    util::net::CSocketAddrFamily,
};

pub(super) fn do_get_route(request_segment: &RouteSegment) -> Result<Vec<RtnlSegment>> {
    let dump_all = GetRequestFlags::from_bits_truncate(request_segment.header().flags)
        .contains(GetRequestFlags::DUMP);

    let mut response_segments = if dump_all {
        // Asterinas does not support NETLINK_GET_STRICT_CHK yet, so dump selectors other than
        // the address family are intentionally ignored. If strict checking is added, validate
        // the rtmsg header fields and attributes, then support selectors such as table, protocol,
        // route type, output interface, and cloned routes.
        let family = route_family_from_request(request_segment);
        route::dump()
            .into_iter()
            .filter(|entry| {
                family.is_none_or(|family| route_family(entry.route().dst().address()) == family)
            })
            .map(|entry| route_to_new_route(request_segment.header(), &entry))
            .map(RtnlSegment::NewRoute)
            .collect()
    } else {
        ensure_full_route_body(request_segment)?;
        ensure_lookup_request(request_segment)?;
        let dst = route_lookup_dst(request_segment)?;
        let lookup_key = RouteLookupKey::new(dst, oif_index(request_segment), None, None, 0, None)?;
        if let Some(oif_index) = lookup_key.oif_index() {
            route::iface_by_index(oif_index).ok_or_else(|| {
                Error::with_message(Errno::ENODEV, "the route output iface does not exist")
            })?;
        }
        let entry = route::lookup(lookup_key)?;
        let route = if request_segment.body().flags.contains(RouteFlags::FIB_MATCH) {
            route_to_new_route(request_segment.header(), &entry)
        } else {
            let source = lookup_route_source(entry.route())?;
            route_to_lookup_route(
                request_segment.header(),
                LookupTableReporting::from_flags(dst, request_segment.body().flags),
                &entry,
                source,
                dst,
            )
        };
        vec![RtnlSegment::NewRoute(route)]
    };

    util::finish_response(request_segment.header(), dump_all, &mut response_segments);
    Ok(response_segments)
}

fn route_to_new_route(request_header: &CMsgSegHdr, entry: &route::RouteTableEntry) -> RouteSegment {
    route_to_new_route_with_flags(request_header, entry, RouteFlags::empty())
}

fn route_to_new_route_with_flags(
    request_header: &CMsgSegHdr,
    entry: &route::RouteTableEntry,
    response_flags: RouteFlags,
) -> RouteSegment {
    let route = entry.route();
    let header = CMsgSegHdr {
        len: 0,
        type_: CSegmentType::NEWROUTE as _,
        flags: SegHdrCommonFlags::empty().bits(),
        seq: request_header.seq,
        pid: request_header.pid,
    };
    let body = RouteSegmentBody {
        family: route_family(route.dst().address()) as _,
        dst_len: route.dst().prefix_len(),
        // TODO: Populate source-prefix and TOS from `RouteEntry` when the route
        // core supports those selectors.
        src_len: 0,
        tos: 0,
        table: Some(entry.table()),
        protocol: route.protocol(),
        scope: route.scope(),
        type_: route.type_(),
        flags: response_flags,
    };
    let mut attrs = Vec::new();
    if route.dst().prefix_len() != 0 {
        attrs.push(RouteAttr::Dst(addr_bytes(route.dst().address())));
    }
    if let Some(gateway) = route.gateway() {
        attrs.push(RouteAttr::Gateway(addr_bytes(gateway)));
    }
    if let Some(oif_index) = route.oif_index() {
        attrs.push(RouteAttr::Oif(oif_index.get()));
    }
    attrs.push(RouteAttr::Table(entry.table().get()));

    RouteSegment::new(header, body, attrs)
}

fn route_to_lookup_route(
    request_header: &CMsgSegHdr,
    table_reporting: LookupTableReporting,
    entry: &route::RouteTableEntry,
    source: IpAddress,
    dst: IpAddress,
) -> RouteSegment {
    let table = table_reporting.response_table(entry);
    let route = entry.route();
    let header = CMsgSegHdr {
        len: 0,
        type_: CSegmentType::NEWROUTE as _,
        flags: SegHdrCommonFlags::empty().bits(),
        seq: request_header.seq,
        pid: request_header.pid,
    };
    let body = RouteSegmentBody {
        family: route_family(dst) as _,
        dst_len: addr_prefix_len(dst),
        // TODO: Populate source-prefix and TOS from `RouteEntry` when the route
        // core supports those selectors.
        src_len: 0,
        tos: 0,
        table: Some(table),
        protocol: match dst {
            IpAddress::Ipv4(_) => RouteProtocol::UNSPEC,
            IpAddress::Ipv6(_) => route.protocol(),
        },
        scope: match dst {
            IpAddress::Ipv4(_) => RouteScope::UNIVERSE,
            IpAddress::Ipv6(_) => route.scope(),
        },
        type_: route.type_(),
        flags: match dst {
            IpAddress::Ipv4(_) => ipv4_lookup_flags(route),
            IpAddress::Ipv6(_) => RouteFlags::empty(),
        },
    };
    let mut attrs = vec![RouteAttr::Dst(addr_bytes(dst))];
    if let Some(gateway) = route.gateway()
        && !route.is_limited_broadcast()
    {
        attrs.push(RouteAttr::Gateway(addr_bytes(gateway)));
    }
    if let Some(oif_index) = route.oif_index() {
        attrs.push(RouteAttr::Oif(oif_index.get()));
    }
    attrs.push(RouteAttr::PrefSrc(addr_bytes(source)));
    if matches!(dst, IpAddress::Ipv4(_)) || table_reporting == LookupTableReporting::Actual {
        attrs.push(RouteAttr::Table(table.get()));
    }

    RouteSegment::new(header, body, attrs)
}

fn ipv4_lookup_flags(route: &RouteEntry) -> RouteFlags {
    let mut flags = RouteFlags::CLONED;
    match route.type_() {
        RouteType::Local => flags |= RouteFlags::IPV4_LOCAL,
        RouteType::Broadcast => flags |= RouteFlags::IPV4_LOCAL | RouteFlags::IPV4_BROADCAST,
        _ => {}
    }
    flags
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum LookupTableReporting {
    /// Reports the table where the lookup route was found.
    Actual,
    /// Reports `RT_TABLE_MAIN` for compatibility with default lookup replies.
    AsMain,
}

impl LookupTableReporting {
    fn from_flags(dst: IpAddress, flags: RouteFlags) -> Self {
        match dst {
            IpAddress::Ipv4(_) if !flags.contains(RouteFlags::LOOKUP_TABLE) => Self::AsMain,
            IpAddress::Ipv4(_) | IpAddress::Ipv6(_) => Self::Actual,
        }
    }

    fn response_table(self, entry: &route::RouteTableEntry) -> RouteTableId {
        match self {
            Self::Actual => entry.table(),
            Self::AsMain => RouteTableId::MAIN,
        }
    }
}

fn lookup_route_source(entry: &RouteEntry) -> Result<IpAddress> {
    let oif_index = entry
        .oif_index()
        .ok_or_else(|| Error::with_message(Errno::ENODEV, "the route has no output iface"))?;
    let iface = route::iface_by_index(oif_index).ok_or_else(|| {
        Error::with_message(Errno::ENODEV, "the route output iface does not exist")
    })?;
    let (source, error_message) = match entry.dst().address() {
        IpAddress::Ipv4(_) => (
            iface
                .ipv4_cidr()
                .map(|cidr| IpAddress::Ipv4(cidr.address())),
            "the route output iface has no IPv4 address",
        ),
        IpAddress::Ipv6(_) => (
            iface
                .ipv6_cidr()
                .map(|cidr| IpAddress::Ipv6(cidr.address())),
            "the route output iface has no IPv6 address",
        ),
    };

    source.ok_or_else(|| Error::with_message(Errno::EADDRNOTAVAIL, error_message))
}

fn ensure_full_route_body(segment: &RouteSegment) -> Result<()> {
    let payload_len = (segment.header().len as usize)
        .checked_sub(RouteSegment::HEADER_LEN)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "the route message length is invalid"))?;
    if payload_len < RouteSegment::BODY_LEN {
        return_errno_with_message!(Errno::EINVAL, "the route message body is too short");
    }

    Ok(())
}

fn ensure_lookup_request(segment: &RouteSegment) -> Result<()> {
    if segment.body().src_len != 0 || segment.body().tos != 0 {
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "source-prefix and TOS lookups are not supported"
        );
    }
    if segment.body().table.is_some()
        || segment.body().scope != RouteScope::UNIVERSE
        || !matches!(segment.body().type_, RouteType::Unspec | RouteType::Unicast)
    {
        return_errno_with_message!(Errno::EINVAL, "the route lookup selector is invalid");
    }
    if segment.body().protocol != RouteProtocol::UNSPEC {
        return_errno_with_message!(Errno::EOPNOTSUPP, "the route protocol is not supported");
    }
    let unsupported_flags = segment.body().flags - RouteFlags::LOOKUP_TABLE - RouteFlags::FIB_MATCH;
    if !unsupported_flags.is_empty() {
        return_errno_with_message!(Errno::EOPNOTSUPP, "the route flags are not supported");
    }
    if segment.attrs().iter().any(|attr| {
        matches!(
            attr,
            RouteAttr::Src(_)
                | RouteAttr::Iif(_)
                | RouteAttr::Gateway(_)
                | RouteAttr::PrefSrc(_)
                | RouteAttr::Priority(_)
                | RouteAttr::Table(_)
        )
    }) {
        return_errno_with_message!(Errno::EOPNOTSUPP, "the route attribute is not supported");
    }

    Ok(())
}

fn route_lookup_dst(segment: &RouteSegment) -> Result<IpAddress> {
    let dst = segment
        .attrs()
        .iter()
        .rev()
        .find_map(|attr| match attr {
            RouteAttr::Dst(dst) => Some(ip_addr_from_bytes(dst)),
            _ => None,
        })
        .transpose()?
        .unwrap_or_else(|| match route_family_from_request(segment) {
            Some(CSocketAddrFamily::AF_INET6) => Ipv6Address::UNSPECIFIED.into(),
            _ => Ipv4Address::UNSPECIFIED.into(),
        });

    if let Some(family) = route_family_from_request(segment)
        && route_family(dst) != family
    {
        return_errno_with_message!(Errno::EINVAL, "the route destination family is invalid");
    }
    if segment.body().dst_len != 0 && segment.body().dst_len != addr_prefix_len(dst) {
        return_errno_with_message!(Errno::EINVAL, "the route destination prefix is invalid");
    }
    Ok(dst)
}

fn oif_index(segment: &RouteSegment) -> Option<NonZeroU32> {
    for attr in segment.attrs().iter().rev() {
        if let RouteAttr::Oif(index) = attr {
            return NonZeroU32::new(*index);
        }
    }

    None
}

fn route_family_from_request(segment: &RouteSegment) -> Option<CSocketAddrFamily> {
    let family = segment.body().family;
    if family == CSocketAddrFamily::AF_INET as i32 {
        Some(CSocketAddrFamily::AF_INET)
    } else if family == CSocketAddrFamily::AF_INET6 as i32 {
        Some(CSocketAddrFamily::AF_INET6)
    } else {
        None
    }
}

fn route_family(addr: IpAddress) -> CSocketAddrFamily {
    match addr {
        IpAddress::Ipv4(_) => CSocketAddrFamily::AF_INET,
        IpAddress::Ipv6(_) => CSocketAddrFamily::AF_INET6,
    }
}

fn addr_prefix_len(addr: IpAddress) -> u8 {
    match addr {
        IpAddress::Ipv4(_) => 32,
        IpAddress::Ipv6(_) => 128,
    }
}

fn addr_bytes(addr: IpAddress) -> Vec<u8> {
    match addr {
        IpAddress::Ipv4(addr) => addr.octets().to_vec(),
        IpAddress::Ipv6(addr) => addr.octets().to_vec(),
    }
}

fn ip_addr_from_bytes(bytes: &[u8]) -> Result<IpAddress> {
    match bytes.len() {
        4 => Ok(Ipv4Address::from_octets(bytes.try_into().unwrap()).into()),
        16 => Ok(Ipv6Address::from_bytes(bytes).into()),
        _ => return_errno_with_message!(Errno::EINVAL, "the route address length is invalid"),
    }
}
