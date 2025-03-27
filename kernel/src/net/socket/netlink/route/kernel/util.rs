// SPDX-License-Identifier: MPL-2.0

use crate::{
    net::socket::netlink::route::message::{
        CMessageSegmentHeader, DoneSegment, NlSegment, SegmentHeaderCommonFlags,
    },
    prelude::*,
};

/// Appends an done segment as the last segment of segments,
/// if needed.
///
/// The done segment will be added if
/// 1. `segments.len()` > 1. Then the segments must be terminated via a done segment;
/// 2. (TODO) The request_header's flags contains `ACK` flag to explicitly request a done segment.
pub fn append_done_segment(request_header: &CMessageSegmentHeader, segments: &mut Vec<NlSegment>) {
    // TODO: How to Respect NetlinkMessageCommonFlags::ACK flag
    if segments.len() <= 1 {
        return;
    }

    let done_segment = DoneSegment::new(request_header, None);
    segments.push(NlSegment::Done(done_segment));
}

/// Adds the `MULTI` flag to all segment in `segments` if `segments.len() > 1`.
pub fn add_multi_flag(segments: &mut Vec<NlSegment>) {
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
