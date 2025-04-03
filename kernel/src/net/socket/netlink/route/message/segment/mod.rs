// SPDX-License-Identifier: MPL-2.0

//! This module defines the message segment,
//! which is the basic unit of a netlink message.
//!
//! Typically, a segment will consist of three parts:
//!
//! 1. Header: The headers of all segments are of type [`CMessageSegmentHeader`],
//! which indicate the type and total length of the segment.
//!
//! 2. Body: The body is the main component of a segment.
//! Each segment will have one and only one body.
//! The body type is defined by the `type_` field of the header.
//!
//! 3. Attributes: Attributes are optional.
//! A segment can have zero or multiple attributes.
//! Attributes belong to different classes,
//! with the class defined by the `type_` field of the header.
//! The total number of attributes is controlled by the `len` field of the header.
//!
//! Note that all headers, bodies, and attributes require
//! their starting address in memory to be aligned to [`super::NLMSG_ALIGN`]
//! when copying to and from user space.
//! Therefore, necessary padding must be added to ensure alignment.
//!
//! The layout of a segment in memory is shown below:
//!
//! ┌────────┬─────────┬──────┬─────────┬──────┬──────┬──────┐
//! │ Header │ Padding │ Body │ Padding │ Attr │ Attr │ Attr │
//! └────────┴─────────┴──────┴─────────┴──────┴──────┴──────┘

pub mod ack;
pub mod addr;
pub mod common;
pub mod header;
mod legacy;
pub mod link;
pub mod route;

use ack::{DoneSegment, ErrorSegment};
use addr::AddrSegment;
use align_ext::AlignExt;
use header::CMessageSegmentHeader;
use legacy::CRtGenMsg;
use link::LinkSegment;

use super::NLMSG_ALIGN;
use crate::prelude::*;

/// The netlink segment, which is the basic unit of a netlink message.
#[derive(Debug)]
pub enum MsgSegment {
    Link(LinkSegment),
    Addr(AddrSegment),
    Done(DoneSegment),
    Error(ErrorSegment),
}

impl MsgSegment {
    pub fn header(&self) -> &CMessageSegmentHeader {
        match self {
            MsgSegment::Link(link_segment) => link_segment.header(),
            MsgSegment::Addr(addr_segment) => addr_segment.header(),
            MsgSegment::Done(done_segment) => done_segment.header(),
            MsgSegment::Error(error_segment) => error_segment.header(),
        }
    }

    pub fn header_mut(&mut self) -> &mut CMessageSegmentHeader {
        match self {
            MsgSegment::Link(link_segment) => link_segment.header_mut(),
            MsgSegment::Addr(addr_segment) => addr_segment.header_mut(),
            MsgSegment::Done(done_segment) => done_segment.header_mut(),
            MsgSegment::Error(error_segment) => error_segment.header_mut(),
        }
    }

    pub fn read_from(reader: &mut VmReader) -> Result<Self> {
        let header = reader.read_val::<CMessageSegmentHeader>()?;

        let segment = match CSegmentType::try_from(header.type_)? {
            CSegmentType::GETLINK => MsgSegment::Link(LinkSegment::read_from(header, reader)?),
            CSegmentType::GETADDR => MsgSegment::Addr(AddrSegment::read_from(header, reader)?),
            _ => return_errno_with_message!(Errno::EINVAL, "unsupported segment type"),
        };

        Ok(segment)
    }

    pub fn write_to(&self, writer: &mut VmWriter) -> Result<()> {
        match self {
            MsgSegment::Link(link_segment) => link_segment.write_to(writer)?,
            MsgSegment::Addr(addr_segment) => addr_segment.write_to(writer)?,
            MsgSegment::Done(done_segment) => done_segment.write_to(writer)?,
            MsgSegment::Error(error_segment) => error_segment.write_to(writer)?,
        }
        Ok(())
    }
}

pub trait SegmentBody: Sized + Clone + Copy {
    type CType: Pod + From<CRtGenMsg> + TryInto<Self> + From<Self>;

    fn read_body_from_user(
        header: &CMessageSegmentHeader,
        reader: &mut VmReader,
    ) -> Result<(Self, usize)>
    where
        Error: From<<Self::CType as TryInto<Self>>::Error>,
    {
        let max_len = header.len as usize - size_of_val(header);

        // The actual message body should be `Self::CType`,
        // but older versions of Linux use a legacy type (usually `CRtGenMsg` here).
        // Additionally, some software, like iproute2, also uses this legacy type.
        // Therefore, we need to handle both cases.
        // Reference: https://elixir.bootlin.com/linux/v6.13/source/net/core/rtnetlink.c#L2393
        // FIXME: Verify whether the legacy type includes any types other than `CRtGenMsg`.
        let (c_type, read_size) = if max_len < size_of::<Self::CType>() {
            let legacy = reader.read_val::<CRtGenMsg>()?;
            // The legacy type cannot be padded.
            // Therefore, there is no need to skip any padding bytes.
            (Self::CType::from(legacy), size_of::<CRtGenMsg>())
        } else {
            let c_type = reader.read_val::<Self::CType>()?;
            let padding_len = Self::padding_len();
            reader.skip(padding_len.min(reader.remain()));
            (c_type, size_of::<Self::CType>() + padding_len)
        };

        let body = c_type.try_into()?;
        Ok((body, read_size))
    }

    fn write_body_to_user(&self, writer: &mut VmWriter) -> Result<()> {
        let c_body = Self::CType::from(*self);
        writer.write_val(&c_body)?;
        let padding_len = Self::padding_len();
        writer.skip(padding_len.min(writer.avail()));
        Ok(())
    }

    fn padding_len() -> usize {
        let payload_len = size_of::<Self::CType>();
        payload_len.align_up(NLMSG_ALIGN) - payload_len
    }
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, TryFromInt, PartialEq, Eq, PartialOrd, Ord)]
pub enum CSegmentType {
    // Standard netlink message types
    NOOP = 1,
    ERROR = 2,
    DONE = 3,
    OVERRUN = 4,

    // protocol-level types
    NEWLINK = 16,
    DELLINK = 17,
    GETLINK = 18,
    SETLINK = 19,

    NEWADDR = 20,
    DELADDR = 21,
    GETADDR = 22,

    NEWROUTE = 24,
    DELROUTE = 25,
    GETROUTE = 26,
    // TODO: The list is not exhaustive.
}
