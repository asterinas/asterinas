// SPDX-License-Identifier: MPL-2.0

//! This module defines the segment that only appears in acknowledgment messages.
//!
//! The acknowledgment segment appears as the final segment of a response message from the kernel.
//! Netlink uses two different classes of acknowledgment segments:
//! 1. The done segment, indicating the termination of a message consisting of multiple segments.
//! 2. The error segment, indicating that an error occurred
//! while the kernel processed the request from user space.
//!

use super::{
    header::{CMessageSegmentHeader, SegmentHeaderCommonFlags},
    CSegmentType, NlMsgSegment,
};
use crate::{
    net::socket::netlink::route::message::{NlAttr, NLMSG_ALIGN},
    prelude::*,
    util::MultiWrite,
};

#[derive(Debug)]
pub struct DoneSegment {
    header: CMessageSegmentHeader,
    error_code: i32,
}

impl DoneSegment {
    pub fn new(request_header: &CMessageSegmentHeader, error: Option<Error>) -> Self {
        let len = size_of::<CMessageSegmentHeader>() + size_of::<i32>();

        let header = CMessageSegmentHeader {
            len: len as _,
            type_: CSegmentType::DONE as _,
            flags: SegmentHeaderCommonFlags::empty().bits(),
            seq: request_header.seq,
            pid: request_header.pid,
        };

        let error_code = error_to_error_code(error);

        Self { header, error_code }
    }
}

impl NlMsgSegment for DoneSegment {
    fn header(&self) -> &CMessageSegmentHeader {
        &self.header
    }

    fn header_mut(&mut self) -> &mut CMessageSegmentHeader {
        &mut self.header
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn attrs(&self) -> &[Box<dyn NlAttr>] {
        &[]
    }

    fn body_len(&self) -> usize {
        size_of_val(&self.error_code)
    }

    fn write_to_user(&self, writer: &mut dyn MultiWrite) -> Result<()> {
        writer.align_to(NLMSG_ALIGN);
        writer.write_val(&self.header)?;
        writer.write_val(&self.error_code)
    }
}

#[derive(Debug)]
pub struct ErrorSegment {
    header: CMessageSegmentHeader,
    error_code: i32,
    request_header: CMessageSegmentHeader,
}

impl ErrorSegment {
    pub fn new(request_header: &CMessageSegmentHeader, error: Option<Error>) -> Self {
        let len = size_of::<CMessageSegmentHeader>() * 2 + size_of::<i32>();

        let header = CMessageSegmentHeader {
            len: len as _,
            type_: CSegmentType::ERROR as _,
            flags: SegmentHeaderCommonFlags::empty().bits(),
            seq: request_header.seq,
            pid: request_header.pid,
        };

        let error_code = error_to_error_code(error);

        Self {
            header,
            error_code,
            request_header: *request_header,
        }
    }
}

impl NlMsgSegment for ErrorSegment {
    fn header(&self) -> &CMessageSegmentHeader {
        &self.header
    }

    fn header_mut(&mut self) -> &mut CMessageSegmentHeader {
        &mut self.header
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn attrs(&self) -> &[Box<dyn NlAttr>] {
        &[]
    }

    fn body_len(&self) -> usize {
        size_of_val(&self.error_code) + size_of_val(&self.request_header)
    }

    fn write_to_user(&self, writer: &mut dyn MultiWrite) -> Result<()> {
        writer.align_to(NLMSG_ALIGN);
        writer.write_val(&self.header)?;
        writer.write_val(&self.error_code)?;
        writer.write_val(&self.request_header)
    }
}

fn error_to_error_code(error: Option<Error>) -> i32 {
    if let Some(error) = error {
        debug!("netlink error: {:?}", error);
        -(error.error() as i32)
    } else {
        0
    }
}
