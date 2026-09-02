// SPDX-License-Identifier: MPL-2.0

//! Handle address-related requests.

use alloc::borrow::ToOwned;
use core::{net::IpAddr, num::NonZeroU32};

use aster_bigtcp::{iface::InterfaceFlags, wire::IpCidr};

use super::util::{self, ResponseMode};
use crate::{
    net::{
        iface::{Iface, iter_all_ifaces},
        socket::netlink::{
            message::{CMsgSegHdr, CSegmentType, GetRequestFlags, SegHdrCommonFlags},
            route::message::{
                AddrAttr, AddrAttrClass, AddrMessageFlags, AddrProtocol, AddrSegment,
                AddrSegmentBody, RtScope, RtnlSegment,
            },
        },
    },
    prelude::*,
    util::net::CSocketAddrFamily,
};

pub(super) fn do_get_addr(
    request_segment: &AddrSegment,
    strict_check: bool,
) -> Result<Vec<RtnlSegment>> {
    let dump_all = {
        let flags = GetRequestFlags::from_bits_truncate(request_segment.header().flags);
        flags.contains(GetRequestFlags::DUMP)
    };
    if !dump_all {
        // Linux's generic `RTM_GETADDR` handler accepts only dump requests,
        // but its IPv6-specific handler also supports single-address queries.
        // TODO: Support single-address IPv6 queries for Linux compatibility.
        // References:
        // - <https://elixir.bootlin.com/linux/v7.1/source/net/core/rtnetlink.c#L7108>
        // - <https://elixir.bootlin.com/linux/v7.1/source/net/ipv6/addrconf.c#L5538>
        return_errno_with_message!(Errno::EOPNOTSUPP, "GETADDR only supports dump requests");
    }

    if strict_check {
        let body = request_segment.body();
        // Linux allows only `IFA_TARGET_NETNSID` in strict address dump requests.
        // Apply the same restriction here,
        // although the attribute parser already rejects `IFA_TARGET_NETNSID`
        // because namespace filtering is not implemented.
        // Reference: <https://elixir.bootlin.com/linux/v7.1/source/net/ipv4/devinet.c#L1811>.
        const ALLOWED: &[AddrAttrClass] = [AddrAttrClass::TARGET_NETNSID].as_slice();
        if request_segment
            .attrs()
            .iter()
            .any(|attr| !ALLOWED.contains(&attr.class()))
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "the attribute is not allowed in an address dump request"
            );
        }
        if let Some(index) = body.index
            && !iter_all_ifaces().any(|iface| iface.index() == index.get())
        {
            return_errno_with_message!(Errno::ENODEV, "the interface does not exist");
        }
    }

    let requested_family = request_segment.body().family;
    // Linux filters address dumps by a nonzero interface index only in strict mode.
    // References:
    // - <https://elixir.bootlin.com/linux/v7.1/source/net/ipv4/devinet.c#L1950>
    // - <https://elixir.bootlin.com/linux/v7.1/source/net/ipv6/addrconf.c#L5430>
    let requested_index = strict_check
        .then(|| request_segment.body().index.map(|index| index.get()))
        .flatten();
    let mut addr_segments: Vec<AddrSegment> = iter_all_ifaces()
        .filter(|iface| requested_index.is_none_or(|index| index == iface.index()))
        .flat_map(|iface| iface_to_new_addrs(request_segment.header(), requested_family, iface))
        .collect();

    // Linux dumps IPv4 addresses before IPv6 addresses
    // while preserving interface order within each family.
    addr_segments.sort_by_key(|segment| segment.body().family);

    let mut response_segments: Vec<RtnlSegment> = addr_segments
        .into_iter()
        .map(RtnlSegment::NewAddr)
        .collect();

    let response_mode = if requested_index.is_some() {
        ResponseMode::FilteredDump
    } else {
        ResponseMode::Dump
    };
    util::finish_response(
        request_segment.header(),
        response_mode,
        &mut response_segments,
    );

    Ok(response_segments)
}

fn iface_to_new_addrs(
    request_header: &CMsgSegHdr,
    requested_family: i32,
    iface: &Arc<Iface>,
) -> impl IntoIterator<Item = AddrSegment> {
    let mut addr_segments = [None, None];

    // Linux dumps addresses for all families
    // when the requested family is neither AF_INET nor AF_INET6.
    let dump_ipv4 = requested_family != CSocketAddrFamily::AF_INET6 as i32;
    let dump_ipv6 = requested_family != CSocketAddrFamily::AF_INET as i32;

    if dump_ipv4 && let Some(cidr) = iface.ipv4_cidr() {
        addr_segments[0] = Some(iface_to_new_addr(request_header, iface, IpCidr::Ipv4(cidr)));
    };

    if dump_ipv6 && let Some(cidr) = iface.ipv6_cidr() {
        addr_segments[1] = Some(iface_to_new_addr(request_header, iface, IpCidr::Ipv6(cidr)));
    }

    addr_segments.into_iter().flatten()
}

fn iface_to_new_addr(
    request_header: &CMsgSegHdr,
    iface: &Arc<Iface>,
    ip_cidr: IpCidr,
) -> AddrSegment {
    let (family, address, prefix_len, scope) = match ip_cidr {
        IpCidr::Ipv4(ipv4_cidr) => {
            let ipv4_addr = ipv4_cidr.address();
            let scope = if ipv4_addr.is_loopback() {
                RtScope::HOST
            } else {
                RtScope::UNIVERSE
            };
            (
                CSocketAddrFamily::AF_INET,
                ipv4_addr.into(),
                ipv4_cidr.prefix_len(),
                scope,
            )
        }
        IpCidr::Ipv6(ipv6_cidr) => {
            let ipv6_addr = ipv6_cidr.address();
            let scope = if ipv6_addr.is_loopback() {
                RtScope::HOST
            } else if ipv6_addr.is_unicast_link_local() {
                RtScope::LINK
            } else {
                RtScope::UNIVERSE
            };
            (
                CSocketAddrFamily::AF_INET6,
                ipv6_addr.into(),
                ipv6_cidr.prefix_len(),
                scope,
            )
        }
    };

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
        scope,
        index: NonZeroU32::new(iface.index()),
    };

    let attrs = match address {
        IpAddr::V4(_) => {
            // Linux may report the following IPv4 address attributes, in order:
            // `IFA_ADDRESS`, `IFA_LOCAL`, `IFA_BROADCAST`, `IFA_LABEL`, `IFA_PROTO`,
            // `IFA_FLAGS`, `IFA_RT_PRIORITY`, and `IFA_CACHEINFO`.
            // TODO: Support `IFA_PROTO`, `IFA_RT_PRIORITY`, and `IFA_CACHEINFO`.
            // Reference: <https://elixir.bootlin.com/linux/v7.1/source/net/ipv4/devinet.c#L1759>.
            let mut attrs = vec![
                // On a point-to-point link, `IFA_ADDRESS` is the peer address.
                // Otherwise, it equals `IFA_LOCAL`.
                // Since `Iface` does not support peer addresses,
                // report the local address here.
                // Reference: <https://elixir.bootlin.com/linux/v7.1/source/include/uapi/linux/if_addr.h#L16>.
                AddrAttr::Address(address),
                AddrAttr::Local(address),
            ];
            if iface.flags().contains(InterfaceFlags::BROADCAST)
                && let Some(broadcast_address) = iface.broadcast_addr()
            {
                attrs.push(AddrAttr::Broadcast(broadcast_address));
            }
            attrs.extend([
                // Linux defaults `IFA_LABEL` to the interface name
                // when no per-address label is configured.
                // Since `Iface` does not support per-address labels,
                // report the interface name.
                // Reference: <https://elixir.bootlin.com/linux/v7.1/source/net/ipv4/devinet.c#L1765>.
                AddrAttr::Label(iface.name().as_cstr().to_owned()),
                AddrAttr::Flags(addr_message.flags),
            ]);
            attrs
        }
        IpAddr::V6(_) => {
            // Linux may report the following IPv6 address attributes, in order:
            // `IFA_LOCAL`, `IFA_ADDRESS`, `IFA_RT_PRIORITY`, `IFA_CACHEINFO`,
            // `IFA_FLAGS`, and `IFA_PROTO`.
            // TODO: Support `IFA_RT_PRIORITY` and `IFA_CACHEINFO`.
            // TODO: Support `IFA_PROTO` for non-loopback addresses.
            // Reference: <https://elixir.bootlin.com/linux/v7.1/source/net/ipv6/addrconf.c#L5143>.
            let mut attrs = vec![
                // When a peer address is configured,
                // `IFA_ADDRESS` is the peer address,
                // and `IFA_LOCAL` is the local address.
                // Otherwise, `IFA_ADDRESS` is the local address,
                // and `IFA_LOCAL` is omitted.
                // Since `Iface` does not support peer addresses,
                // report only `IFA_ADDRESS` with the local address.
                // Reference: <https://elixir.bootlin.com/linux/v7.1/source/net/ipv6/addrconf.c#L5189>.
                AddrAttr::Address(address),
                AddrAttr::Flags(addr_message.flags),
            ];
            if address.is_loopback() {
                // Linux marks an IPv6 loopback address with `IFAPROT_KERNEL_LO`.
                // Reference: <https://elixir.bootlin.com/linux/v7.1/source/net/ipv6/addrconf.c#L3289>.
                attrs.push(AddrAttr::Protocol(AddrProtocol::KernelLoopback));
            }
            attrs
        }
    };

    AddrSegment::new(header, addr_message, attrs)
}
