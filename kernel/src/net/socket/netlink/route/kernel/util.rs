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
    append_done_segment(request_header, dump_all, response_segments);
    add_multi_flag(response_segments);
}

/// Appends a done segment as the last segment of the provided segments, if necessary.
///
/// A done segment will be added if:
/// 1. `segments.len()` > 1, as mulitible segments must be terminated with a done segment.
/// 2. `segments.len()` is 1, and `dump_all` is true. (FIXME: Is this true for all dump requests?)
/// 2. (TODO) The request_header's flags contain the `ACK` flag, explicitly requesting a done segment.
fn append_done_segment(
    request_header: &CMsgSegHdr,
    dump_all: bool,
    response_segments: &mut Vec<RtnlSegment>,
) {
    // TODO: Deal with the `NetlinkMessageCommonFlags::ACK` flag.

    if response_segments.len() == 0 {
        todo!("how to deal with ")
    }

    if response_segments.len() == 1 && !dump_all {
        return;
    }

    let done_segment = DoneSegment::new_from_request(request_header, None);
    response_segments.push(RtnlSegment::Done(done_segment));
}

/// Adds the `MULTI` flag to all segments in `segments` if `segments.len() > 1`.
fn add_multi_flag(response_segments: &mut Vec<RtnlSegment>) {
    if response_segments.len() <= 1 {
        return;
    }

    for segment in response_segments.iter_mut() {
        let header = segment.header_mut();
        let mut flags = SegHdrCommonFlags::from_bits_truncate(header.flags);
        flags |= SegHdrCommonFlags::MULTI;
        header.flags = flags.bits();
    }
}
