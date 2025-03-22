// SPDX-License-Identifier: MPL-2.0

//! Deal with link-related requests.

use core::num::{NonZero, NonZeroU32};

use super::util::{add_multi_flag_if_required, append_ack_segment};
use crate::{
    net::{
        iface::{ConfigurableIface, IFACES},
        socket::netlink::route::message::{
            attr::link::{IflaName, IflaTxqLen},
            CMessageSegmentHeader, CSegmentType, GetRequestFlags, LinkSegment, LinkSegmentBody,
            NlAttr, NlMsg, NlMsgSegment, ReadAttrFromUser, ReadNlMsgSegmentFromUser,
            SegmentHeaderCommonFlags,
        },
    },
    prelude::*,
};

pub fn do_get_link(segment: &dyn NlMsgSegment) -> NlMsg {
    let request_segment = segment.as_any().downcast_ref::<LinkSegment>().unwrap();
    let flags = GetRequestFlags::from_bits_truncate(request_segment.header().flags);

    let ifaces = IFACES.get().unwrap();
    let mut response_segments = ifaces
        .iter()
        .filter(|iface| {
            if flags.contains(GetRequestFlags::DUMP) {
                return true;
            }

            if let Some(required_index) = request_segment.body().index.map(NonZeroU32::get) {
                if required_index != *iface.index() {
                    return false;
                }
            }

            if let Some(if_name) = request_segment
                .attrs()
                .iter()
                .filter_map(|attr| attr.as_any().downcast_ref::<IflaName>())
                .nth(0)
            {
                let required_name = if_name.value.to_str().unwrap();
                if required_name != iface.name().as_str() {
                    return false;
                }
            }

            true
        })
        .map(|iface| iface_to_new_link(request_segment.header(), iface))
        .map(|link_segment| Box::new(link_segment) as Box<dyn NlMsgSegment>)
        .collect::<Vec<_>>();

    append_ack_segment(request_segment.header(), &mut response_segments);
    add_multi_flag_if_required(&mut response_segments);

    NlMsg::new(response_segments)
}

fn iface_to_new_link(
    request_header: &CMessageSegmentHeader,
    iface: &ConfigurableIface,
) -> LinkSegment {
    let header = CMessageSegmentHeader {
        len: 0,
        type_: CSegmentType::NEWLINK as _,
        flags: SegmentHeaderCommonFlags::empty().bits(),
        seq: request_header.seq,
        pid: request_header.pid,
    };

    let link_message = LinkSegmentBody {
        family: *iface.family(),
        type_: *iface.type_(),
        index: NonZero::new(*iface.index()),
        flags: *iface.flags(),
    };

    let attrs = vec![
        Box::new(IflaName::new(CString::new(iface.name().as_str()).unwrap())) as Box<dyn NlAttr>,
        Box::new(IflaTxqLen::new((*iface.txqlen()) as u32)),
    ];

    LinkSegment::new(header, link_message, attrs)
}
