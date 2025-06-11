// SPDX-License-Identifier: MPL-2.0

//! This module defines the message segment,
//! which is the basic unit of a netlink message.
//!
//! Typically, a segment will consist of three parts:
//!
//! 1. Header: The headers of all segments are of type [`CMegSegHdr`],
//!    which indicate the type and total length of the segment.
//!
//! 2. Body: The body is the main component of a segment.
//!    Each segment will have one and only one body.
//!    The body type is defined by the `type_` field of the header.
//!
//! 3. Attributes: Attributes are optional.
//!    A segment can have zero or multiple attributes.
//!    Attributes belong to different classes,
//!    with the class defined by the `type_` field of the header.
//!    The total number of attributes is controlled by the `len` field of the header.
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

pub mod addr;
mod legacy;
pub mod link;
pub mod route;

use addr::AddrSegment;
use link::LinkSegment;

use crate::{
    net::socket::netlink::message::{
        CMsgSegHdr, CSegmentType, DoneSegment, ErrorSegment, ProtocolSegment,
    },
    prelude::*,
    util::{MultiRead, MultiWrite},
};

/// The netlink route segment, which is the basic unit of a netlink route message.
#[derive(Debug)]
pub enum RtnlSegment {
    NewLink(LinkSegment),
    GetLink(LinkSegment),
    NewAddr(AddrSegment),
    GetAddr(AddrSegment),
    Done(DoneSegment),
    Error(ErrorSegment),
}

impl ProtocolSegment for RtnlSegment {
    fn header(&self) -> &CMsgSegHdr {
        match self {
            RtnlSegment::NewLink(link_segment) | RtnlSegment::GetLink(link_segment) => {
                link_segment.header()
            }
            RtnlSegment::NewAddr(addr_segment) | RtnlSegment::GetAddr(addr_segment) => {
                addr_segment.header()
            }
            RtnlSegment::Done(done_segment) => done_segment.header(),
            RtnlSegment::Error(error_segment) => error_segment.header(),
        }
    }

    fn header_mut(&mut self) -> &mut CMsgSegHdr {
        match self {
            RtnlSegment::NewLink(link_segment) | RtnlSegment::GetLink(link_segment) => {
                link_segment.header_mut()
            }
            RtnlSegment::NewAddr(addr_segment) | RtnlSegment::GetAddr(addr_segment) => {
                addr_segment.header_mut()
            }
            RtnlSegment::Done(done_segment) => done_segment.header_mut(),
            RtnlSegment::Error(error_segment) => error_segment.header_mut(),
        }
    }

    fn read_from(reader: &mut dyn MultiRead) -> Result<Self> {
        let header = reader
            .read_val_opt::<CMsgSegHdr>()?
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "the reader length is too small"))?;

        let segment = match CSegmentType::try_from(header.type_)? {
            CSegmentType::GETLINK => RtnlSegment::GetLink(LinkSegment::read_from(header, reader)?),
            CSegmentType::GETADDR => RtnlSegment::GetAddr(AddrSegment::read_from(header, reader)?),
            _ => return_errno_with_message!(Errno::EINVAL, "unsupported segment type"),
        };

        Ok(segment)
    }

    fn write_to(&self, writer: &mut dyn MultiWrite) -> Result<()> {
        match self {
            RtnlSegment::NewLink(link_segment) => link_segment.write_to(writer)?,
            RtnlSegment::NewAddr(addr_segment) => addr_segment.write_to(writer)?,
            RtnlSegment::Done(done_segment) => done_segment.write_to(writer)?,
            RtnlSegment::Error(error_segment) => error_segment.write_to(writer)?,
            RtnlSegment::GetAddr(_) | RtnlSegment::GetLink(_) => {
                unreachable!("kernel should not write get requests to user space");
            }
        }
        Ok(())
    }
}
