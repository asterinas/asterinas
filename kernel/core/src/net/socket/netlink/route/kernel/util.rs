// SPDX-License-Identifier: MPL-2.0

use crate::{
    net::socket::netlink::{
        message::{CMsgSegHdr, DoneSegment, ProtocolSegment, SegHdrCommonFlags},
        route::message::RtnlSegment,
    },
    prelude::*,
};

#[derive(Eq, PartialEq)]
pub(super) enum ResponseMode {
    /// A single response without a `DONE` segment.
    Single,
    /// A multipart dump ending with a `DONE` segment.
    Dump,
    /// A dump with `DUMP_FILTERED` set on every segment, including `DONE`.
    FilteredDump,
}

/// Finishes a response message.
pub(super) fn finish_response(
    request_header: &CMsgSegHdr,
    mode: ResponseMode,
    response_segments: &mut Vec<RtnlSegment>,
) {
    let response_flags = match mode {
        ResponseMode::Single => {
            assert_eq!(response_segments.len(), 1);
            return;
        }
        ResponseMode::Dump => SegHdrCommonFlags::MULTI,
        ResponseMode::FilteredDump => SegHdrCommonFlags::MULTI | SegHdrCommonFlags::DUMP_FILTERED,
    };

    append_done_segment(request_header, response_segments);
    for segment in response_segments.iter_mut() {
        let header = segment.header_mut();
        let mut flags = SegHdrCommonFlags::from_bits_truncate(header.flags);
        flags |= response_flags;
        header.flags = flags.bits();
    }
}

/// Appends a done segment as the last segment of the provided segments.
fn append_done_segment(request_header: &CMsgSegHdr, response_segments: &mut Vec<RtnlSegment>) {
    let done_segment = DoneSegment::new_from_request(request_header, None);
    response_segments.push(RtnlSegment::Done(done_segment));
}
