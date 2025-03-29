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
    CSegmentType, NlSegmentCommonOps,
};
use crate::{net::socket::netlink::route::message::util::align_writer, prelude::*};

#[derive(Debug)]
pub struct DoneSegment {
    header: CMessageSegmentHeader,
    error_code: i32,
}

impl DoneSegment {
    pub fn new(request_header: &CMessageSegmentHeader, error: Option<Error>) -> Self {
        let len = Self::HEADER_LEN + Self::BODY_LEN;

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

impl NlSegmentCommonOps for DoneSegment {
    const BODY_LEN: usize = size_of::<i32>();

    fn header(&self) -> &CMessageSegmentHeader {
        &self.header
    }

    fn header_mut(&mut self) -> &mut CMessageSegmentHeader {
        &mut self.header
    }

    fn attrs_len(&self) -> usize {
        0
    }

    fn read_from(_header: CMessageSegmentHeader, _reader: &mut VmReader) -> Result<Self>
    where
        Self: Sized,
    {
        return_errno_with_message!(Errno::EINVAL, "done segment should not be read from user");
    }

    fn write_to(&self, writer: &mut VmWriter) -> Result<()> {
        align_writer(writer)?;
        writer.write_val(&self.header)?;
        writer.write_val(&self.error_code)?;
        Ok(())
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
        let len = Self::HEADER_LEN + Self::BODY_LEN;

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

impl NlSegmentCommonOps for ErrorSegment {
    const BODY_LEN: usize = size_of::<i32>() + size_of::<CMessageSegmentHeader>();

    fn header(&self) -> &CMessageSegmentHeader {
        &self.header
    }

    fn header_mut(&mut self) -> &mut CMessageSegmentHeader {
        &mut self.header
    }

    fn attrs_len(&self) -> usize {
        0
    }

    fn read_from(_header: CMessageSegmentHeader, _reader: &mut VmReader) -> Result<Self>
    where
        Self: Sized,
    {
        return_errno_with_message!(Errno::EINVAL, "error segment should not be read from user");
    }

    fn write_to(&self, writer: &mut VmWriter) -> Result<()> {
        align_writer(writer)?;
        writer.write_val(&self.header)?;
        writer.write_val(&self.error_code)?;
        writer.write_val(&self.request_header)?;
        Ok(())
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
