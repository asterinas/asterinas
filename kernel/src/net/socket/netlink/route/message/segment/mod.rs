// SPDX-License-Identifier: MPL-2.0

//! This module defines the message segment, which is the basic unit of a netlink message.
//!
//! Typically, a segment will have three parts:
//!
//! 1. Header. The headers of all segments are of type [`CMessageSegmentHeader`],
//! which indicate the type and total length of the segment.
//!
//! 2. Body. The body is the main part of a segment.
//! Each segment will have one and only one body.
//! The body type is defined by the `type_` field of the header.
//!
//! 3. Attributes. Attributes are optional.
//! A segment may have zero or multiple attributes.
//! Attributes belong to different classes,
//! and the class is also defined by the `type_` field of the header.
//! The total number of attributes is controlled by the `len` field of the header.
//!
//! Note that all headers, bodies, and attributes require
//! their starting address in memory to be aligned to [`super::NLMSG_ALIGN`]
//! when copying to and from user space.
//! Therefore, some necessary padding must be added to ensure the alignment.
//!
//! The layout of a segment in memory is shown below:
//!
//! ┌────────┬─────────┬──────┬─────────┬──────┬──────┬──────┐
//! │ Header │ Padding │ Body │ Padding │ Attr │ Attr │ Attr │
//! └────────┴─────────┴──────┴─────────┴──────┴──────┴──────┘

pub mod ack;
pub mod addr;
pub mod header;
mod legacy;
pub mod link;
pub mod route;

use ack::{DoneSegment, ErrorSegment};
use addr::AddrSegment;
use align_ext::AlignExt;
use header::CMessageSegmentHeader;
use link::LinkSegment;

use super::{util::align_writer, NLMSG_ALIGN};
use crate::prelude::*;

/// The netlink segment, which is the basic unit of a netlink message.
#[derive(Debug)]
pub enum NlSegment {
    Link(LinkSegment),
    Addr(AddrSegment),
    Done(DoneSegment),
    Error(ErrorSegment),
}

impl NlSegment {
    pub fn header(&self) -> &CMessageSegmentHeader {
        match self {
            NlSegment::Link(link_segment) => link_segment.header(),
            NlSegment::Addr(addr_segment) => addr_segment.header(),
            NlSegment::Done(done_segment) => done_segment.header(),
            NlSegment::Error(error_segment) => error_segment.header(),
        }
    }

    pub fn header_mut(&mut self) -> &mut CMessageSegmentHeader {
        match self {
            NlSegment::Link(link_segment) => link_segment.header_mut(),
            NlSegment::Addr(addr_segment) => addr_segment.header_mut(),
            NlSegment::Done(done_segment) => done_segment.header_mut(),
            NlSegment::Error(error_segment) => error_segment.header_mut(),
        }
    }

    pub fn read_from(reader: &mut VmReader) -> Result<Self> {
        let header = reader.read_val::<CMessageSegmentHeader>()?;

        let segment = match CSegmentType::try_from(header.type_)? {
            CSegmentType::GETLINK => NlSegment::Link(LinkSegment::read_from(header, reader)?),
            CSegmentType::GETADDR => NlSegment::Addr(AddrSegment::read_from(header, reader)?),
            _ => todo!("support other segments"),
        };

        Ok(segment)
    }

    pub fn write_to(&self, writer: &mut VmWriter) -> Result<()> {
        match self {
            NlSegment::Link(link_segment) => link_segment.write_to(writer)?,
            NlSegment::Addr(addr_segment) => addr_segment.write_to(writer)?,
            NlSegment::Done(done_segment) => done_segment.write_to(writer)?,
            NlSegment::Error(error_segment) => error_segment.write_to(writer)?,
        }
        align_writer(writer)?;
        Ok(())
    }
}
/// The common operations defined on a netlink segment.
pub trait NlSegmentCommonOps: Send + Sync + Debug {
    const HEADER_LEN: usize = size_of::<CMessageSegmentHeader>();
    const BODY_LEN: usize;

    fn header(&self) -> &CMessageSegmentHeader;
    fn header_mut(&mut self) -> &mut CMessageSegmentHeader;
    fn attrs_len(&self) -> usize;
    fn total_len(&self) -> usize {
        Self::HEADER_LEN.align_up(NLMSG_ALIGN)
            + Self::BODY_LEN.align_up(NLMSG_ALIGN)
            + self.attrs_len().align_up(NLMSG_ALIGN)
    }
    fn read_from(header: CMessageSegmentHeader, reader: &mut VmReader) -> Result<Self>
    where
        Self: Sized;
    fn write_to(&self, writer: &mut VmWriter) -> Result<()>;
}

pub trait SegmentBody: Sized + Clone + Copy {
    // The actual message body should be `Self::CType`,
    // but older versions of Linux use a legacy type (usually `CRtGenMessage`) here.
    // Additionally, some software, like iproute2, also uses this legacy type.
    // Therefore, we need to handle both cases.
    // Reference: https://elixir.bootlin.com/linux/v6.13/source/net/core/rtnetlink.c#L2393
    // FIXME: We need to verify whether the legacy type includes any types other than `CRtGenMessage`.
    // If it does not, the associated generic `LegacyType` can be removed.
    type LegacyType: Pod;
    type CType: Pod + From<Self::LegacyType> + TryInto<Self> + From<Self>;

    fn read_body_from_user(
        header: &CMessageSegmentHeader,
        reader: &mut VmReader,
    ) -> Result<(Self, usize)>
    where
        Error: From<<Self::CType as TryInto<Self>>::Error>,
    {
        let max_len = header.len as usize - size_of_val(header);

        let (c_type, read_size) = if max_len < size_of::<Self::CType>() {
            let legacy = reader.read_val::<Self::LegacyType>()?;
            (Self::CType::from(legacy), size_of::<Self::LegacyType>())
        } else {
            let c_type = reader.read_val::<Self::CType>()?;
            (c_type, size_of::<Self::CType>())
        };

        let body = c_type.try_into()?;
        Ok((body, read_size))
    }

    fn write_body_to_user(&self, writer: &mut VmWriter) -> Result<()> {
        let c_body = Self::CType::from(*self);
        writer.write_val(&c_body)?;
        Ok(())
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
    // TODO: The list is not exhaustive now.
}
