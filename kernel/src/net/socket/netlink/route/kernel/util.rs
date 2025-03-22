// SPDX-License-Identifier: MPL-2.0

use crate::{
    net::socket::netlink::route::message::{
        CMessageSegmentHeader, DoneSegment, ErrorSegment, NlMsgSegment, SegmentHeaderCommonFlags,
    },
    prelude::*,
};

/// Appends an ack segment as the last segment of segments, if needed.
//
// FIXME: The current logic for adding error segments only handles GET requests.
// If the `segments` are empty, we simply add an ENODEV error segment.
// Once we support other types of requests, we should implement more general error handling.
pub fn append_ack_segment(
    request_header: &CMessageSegmentHeader,
    segments: &mut Vec<Box<dyn NlMsgSegment>>,
) {
    if segments.len() == 1 {
        // FIXME: Respect NetlinkMessageCommonFlags::ACK flag
        return;
    }

    if segments.len() > 1 {
        let done_segment = DoneSegment::new(request_header, None);
        segments.push(Box::new(done_segment));
        return;
    }

    let error_segment = ErrorSegment::new(request_header, Some(Errno::ENODEV.into()));
    segments.push(Box::new(error_segment));
}

pub fn add_multi_flag_if_required(segments: &mut Vec<Box<dyn NlMsgSegment>>) {
    if segments.len() <= 1 {
        return;
    }

    for segment in segments.iter_mut() {
        let header = segment.header_mut();
        let mut flags = SegmentHeaderCommonFlags::from_bits_truncate(header.flags);
        flags |= SegmentHeaderCommonFlags::MULTI;
        header.flags = flags.bits();
    }
}
