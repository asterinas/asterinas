// SPDX-License-Identifier: MPL-2.0

use crate::{
    net::socket::netlink::route::message::{
        CMessageSegmentHeader, DoneSegment, MsgSegment, SegmentHeaderCommonFlags,
    },
    prelude::*,
};

/// Appends a done segment as the last segment of the provided segments, if necessary.
///
/// A done segment will be added if:
/// 1. `segments.len()` > 1, as mulitible segments must be terminated with a done segment.
/// 2. (TODO) The request_header's flags contain the `ACK` flag, explicitly requesting a done segment.
pub fn append_done_segment(request_header: &CMessageSegmentHeader, segments: &mut Vec<MsgSegment>) {
    // TODO: Deal with the `NetlinkMessageCommonFlags::ACK` flag
    if segments.len() <= 1 {
        return;
    }

    let done_segment = DoneSegment::new_from_request(request_header, None);
    segments.push(MsgSegment::Done(done_segment));
}

/// Adds the `MULTI` flag to all segments in `segments` if `segments.len() > 1`.
pub fn add_multi_flag(segments: &mut Vec<MsgSegment>) {
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
