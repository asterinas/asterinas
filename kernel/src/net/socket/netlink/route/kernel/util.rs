// SPDX-License-Identifier: MPL-2.0

use crate::{
    net::socket::netlink::{
        message::{CMsgSegHdr, DoneSegment, ProtocolSegment, SegHdrCommonFlags},
        route::message::RtnlSegment,
    },
    prelude::*,
};

/// Finishes a response message.
pub fn finish_response(
    request_header: &CMsgSegHdr,
    dump_all: bool,
    response_segments: &mut Vec<RtnlSegment>,
) {
    if !dump_all {
        assert_eq!(response_segments.len(), 1);
        return;
    }
    append_done_segment(request_header, response_segments);
    add_multi_flag(response_segments);
}

/// Appends a done segment as the last segment of the provided segments.
fn append_done_segment(request_header: &CMsgSegHdr, response_segments: &mut Vec<RtnlSegment>) {
    let done_segment = DoneSegment::new_from_request(request_header, None);
    response_segments.push(RtnlSegment::Done(done_segment));
}

/// Adds the `MULTI` flag to all segments in `segments`.
fn add_multi_flag(response_segments: &mut [RtnlSegment]) {
    for segment in response_segments.iter_mut() {
        let header = segment.header_mut();
        let mut flags = SegHdrCommonFlags::from_bits_truncate(header.flags);
        flags |= SegHdrCommonFlags::MULTI;
        header.flags = flags.bits();
    }
}
