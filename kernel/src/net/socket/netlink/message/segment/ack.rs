// SPDX-License-Identifier: MPL-2.0

//! This module defines segments that only appear in acknowledgment messages.
//!
//! An acknowledgment segment appears as the final segment in a response message from the kernel.
//! Netlink utilizes two classes of acknowledgment segments:
//! 1. Done Segment: Indicates the conclusion of a message comprised of multiple segments.
//! 2. Error Segment: Indicates that an error occurred while the kernel processed the user space request.
//!

use super::{
    common::SegmentCommon,
    header::{CMsgSegHdr, SegHdrCommonFlags},
    CSegmentType, SegmentBody,
};
use crate::{net::socket::netlink::message::NoAttr, prelude::*};

pub type DoneSegment = SegmentCommon<DoneSegmentBody, NoAttr>;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DoneSegmentBody {
    error_code: i32,
}

impl SegmentBody for DoneSegmentBody {
    type CType = DoneSegmentBody;
}

impl DoneSegment {
    pub fn new_from_request(request_header: &CMsgSegHdr, error: Option<Error>) -> Self {
        let header = CMsgSegHdr {
            len: 0,
            type_: CSegmentType::DONE as _,
            flags: SegHdrCommonFlags::empty().bits(),
            seq: request_header.seq,
            pid: request_header.pid,
        };

        let body = {
            let error_code = error_to_error_code(error);
            DoneSegmentBody { error_code }
        };

        Self::new(header, body, Vec::new())
    }
}

pub type ErrorSegment = SegmentCommon<ErrorSegmentBody, NoAttr>;

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct ErrorSegmentBody {
    error_code: i32,
    request_header: CMsgSegHdr,
}

impl SegmentBody for ErrorSegmentBody {
    type CType = ErrorSegmentBody;
}

impl ErrorSegment {
    pub fn new_from_request(request_header: &CMsgSegHdr, error: Option<Error>) -> Self {
        let header = CMsgSegHdr {
            len: 0,
            type_: CSegmentType::ERROR as _,
            flags: SegHdrCommonFlags::empty().bits(),
            seq: request_header.seq,
            pid: request_header.pid,
        };

        let body = {
            let error_code = error_to_error_code(error);
            ErrorSegmentBody {
                error_code,
                request_header: *request_header,
            }
        };

        Self::new(header, body, Vec::new())
    }
}

const fn error_to_error_code(error: Option<Error>) -> i32 {
    if let Some(error) = error {
        -(error.error() as i32)
    } else {
        0
    }
}
