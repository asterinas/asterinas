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

use align_ext::AlignExt;
use header::CMessageSegmentHeader;

use super::NlAttr;
use crate::{
    prelude::*,
    util::{MultiRead, MultiWrite},
};

/// A netlink message segment.
///
/// The details of the segment can be viewed in the module doc.
pub trait NlMsgSegment: Send + Sync + Debug {
    fn header(&self) -> &CMessageSegmentHeader;
    fn header_mut(&mut self) -> &mut CMessageSegmentHeader;
    fn body_len(&self) -> usize;
    fn attrs(&self) -> &[Box<dyn NlAttr>];
    fn total_len(&self) -> usize {
        let attrs_len: usize = self
            .attrs()
            .iter()
            .map(|attr| attr.total_len_with_padding())
            .sum();
        size_of::<CMessageSegmentHeader>() + self.body_len() + attrs_len
    }
    fn write_to_user(&self, writer: &mut dyn MultiWrite) -> Result<()>;
    fn as_any(&self) -> &dyn Any;
}

pub trait ReadNlMsgSegmentFromUser: Sized {
    type Body: ReadBodyFromUser;

    fn new(header: CMessageSegmentHeader, body: Self::Body, attrs: Vec<Box<dyn NlAttr>>) -> Self;

    fn read_from_user(header: CMessageSegmentHeader, reader: &mut dyn MultiRead) -> Result<Self>
    where
        Error: From<
            <<<Self as ReadNlMsgSegmentFromUser>::Body as ReadBodyFromUser>::CType as TryInto<
                <Self as ReadNlMsgSegmentFromUser>::Body,
            >>::Error,
        >,
    {
        let (body, body_len) =
            <Self::Body as ReadBodyFromUser>::read_body_from_user(&header, reader)?;

        let attrs = {
            let attrs_len = (header.len as usize - size_of_val(&header) - body_len).align_down(4);
            Self::read_attrs(attrs_len, reader)?
        };

        let segment = Self::new(header, body, attrs);
        Ok(segment)
    }

    fn read_attrs(attrs_len: usize, reader: &mut dyn MultiRead) -> Result<Vec<Box<dyn NlAttr>>>;
}

pub trait ReadBodyFromUser: Sized + Clone + Copy {
    // The actual message body should be `Self::CType`,
    // but older versions of Linux use a legacy type (usually `CRtGenMessage`) here.
    // Additionally, some software, like iproute2, also uses this legacy type.
    // Therefore, we need to handle both cases.
    // Reference: https://elixir.bootlin.com/linux/v6.13/source/net/core/rtnetlink.c#L2393
    // FIXME: We need to verify whether the legacy type includes any types other than `CRtGenMessage`.
    // If it does not, the associated generic `LegacyType` can be removed.
    type LegacyType: Pod;
    type CType: Pod + From<Self::LegacyType> + TryInto<Self> + From<Self>;

    fn validate_read_value(_header: &CMessageSegmentHeader, _c_type: &Self::CType) -> Result<()> {
        Ok(())
    }

    fn read_body_from_user(
        header: &CMessageSegmentHeader,
        reader: &mut dyn MultiRead,
    ) -> Result<(Self, usize)>
    where
        Error: From<<<Self as ReadBodyFromUser>::CType as TryInto<Self>>::Error>,
    {
        let max_len = header.len as usize - size_of_val(header);

        let (c_type, read_size) = if max_len < size_of::<Self::CType>() {
            let legacy = reader.read_val::<Self::LegacyType>()?;
            (Self::CType::from(legacy), size_of::<Self::LegacyType>())
        } else {
            let c_type = reader.read_val::<Self::CType>()?;
            (c_type, size_of::<Self::CType>())
        };

        Self::validate_read_value(header, &c_type)?;

        let body = c_type.try_into()?;
        Ok((body, read_size))
    }
}

pub trait WriteBodyToUser: ReadBodyFromUser {
    fn write_body_to_user(&self, writer: &mut dyn MultiWrite) -> Result<()> {
        let c_body = <Self as ReadBodyFromUser>::CType::from(*self);
        writer.write_val(&c_body)
    }
}

/// This macro will implements [`NlMsgSegment`] and [`ReadNlMsgSegmentFromUser`] for a segment type.
macro_rules! impl_nlsegment_general {
    ($segment_type: ty, $body_type: ty, $body_c_type: ty, $read_attr:ident) => {
        impl NlMsgSegment for $segment_type {
            fn header(&self) -> &CMessageSegmentHeader {
                &self.header
            }

            fn header_mut(&mut self) -> &mut CMessageSegmentHeader {
                &mut self.header
            }

            fn as_any(&self) -> &dyn Any {
                self
            }

            fn body_len(&self) -> usize {
                size_of::<$body_c_type>()
            }

            fn attrs(&self) -> &[Box<dyn NlAttr>] {
                &self.attrs
            }

            fn write_to_user(&self, writer: &mut dyn MultiWrite) -> Result<()> {
                writer.align_to(NLMSG_ALIGN);
                writer.write_val(&self.header)?;
                self.body.write_body_to_user(writer)?;
                for attr in self.attrs() {
                    attr.write_attr_to_user(writer)?;
                }

                Ok(())
            }
        }

        impl ReadNlMsgSegmentFromUser for $segment_type {
            type Body = $body_type;

            fn new(
                header: CMessageSegmentHeader,
                body: Self::Body,
                attrs: Vec<Box<dyn NlAttr>>,
            ) -> Self {
                let mut segment = Self {
                    header,
                    body,
                    attrs,
                };

                if header.len == 0 {
                    segment.header.len = segment.total_len() as _;
                }

                segment
            }

            fn read_attrs(
                attrs_len: usize,
                reader: &mut dyn MultiRead,
            ) -> Result<Vec<Box<dyn NlAttr>>> {
                $read_attr(attrs_len, reader)
            }
        }
    };
}

use impl_nlsegment_general;

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
