// SPDX-License-Identifier: MPL-2.0

//! Deal with address-related requests.

use core::num::NonZeroU32;

use super::util::{add_multi_flag_if_required, append_ack_segment};
use crate::{
    net::{
        iface::{ConfigurableIface, IFACES},
        socket::netlink::route::message::{
            attr::addr::{IfaAddress, IfaLabel, IfaLocal},
            AddrMessageFlags, AddrSegment, AddrSegmentBody, CMessageSegmentHeader, CSegmentType,
            GetRequestFlags, NlAttr, NlMsg, NlMsgSegment, ReadAttrFromUser,
            ReadNlMsgSegmentFromUser, RtScope, SegmentHeaderCommonFlags,
        },
    },
    prelude::*,
    util::net::CSocketAddrFamily,
};

pub fn do_get_addr(segment: &dyn NlMsgSegment) -> NlMsg {
    let request_segment = segment.as_any().downcast_ref::<AddrSegment>().unwrap();
    let flags = GetRequestFlags::from_bits_truncate(request_segment.header().flags);

    let ifaces = IFACES.get().unwrap();
    let mut response_segments = ifaces
        .iter()
        .filter(|iface| {
            // Filter only requested addresses

            if iface.iface().ipv4_addr().is_none() {
                return false;
            }

            if flags.contains(GetRequestFlags::DUMP) {
                return true;
            }

            if let Some(index) = request_segment.body().index.map(NonZeroU32::get) {
                if *iface.index() != index {
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
        .map(|addr_segment| Box::new(addr_segment) as Box<dyn NlMsgSegment>)
        .collect::<Vec<_>>();

    append_ack_segment(request_segment.header(), &mut response_segments);
    add_multi_flag_if_required(&mut response_segments);

    NlMsg::new(response_segments)
}

fn iface_to_new_addr(
    request_header: &CMessageSegmentHeader,
    iface: &ConfigurableIface,
) -> AddrSegment {
    let ipv4_addr = iface.iface().ipv4_addr().unwrap();

    let header = CMessageSegmentHeader {
        len: 0,
        type_: CSegmentType::NEWADDR as _,
        flags: SegmentHeaderCommonFlags::empty().bits(),
        seq: request_header.seq,
        pid: request_header.pid,
    };

    let addr_message = AddrSegmentBody {
        family: CSocketAddrFamily::AF_INET,
        prefix_len: iface.iface().prefix_len().unwrap(),
        flags: AddrMessageFlags::PERMANENT,
        scope: RtScope::HOST,
        index: NonZeroU32::new(*iface.index()),
    };

    let attrs = vec![
        Box::new(IfaAddress::new(ipv4_addr.octets())) as Box<dyn NlAttr>,
        Box::new(IfaLabel::new(CString::new(iface.name().as_str()).unwrap())),
        Box::new(IfaLocal::new(ipv4_addr.octets())),
    ];

    AddrSegment::new(header, addr_message, attrs)
}
