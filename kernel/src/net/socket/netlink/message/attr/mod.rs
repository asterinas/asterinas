// SPDX-License-Identifier: MPL-2.0

//! Netlink attributes.
//!
//! Netlink attributes provide additional information for each [`segment`].
//! Each netlink attribute consists of two components:
//! 1. Header: The attribute header is of type [`CNlAttrHeader`],
//!    which specifies the type and length of the attribute. The attribute
//!    type belongs to different classes, which rely on the segment type.
//! 2. Payload: The attribute's payload, which can vary in type.
//!    Currently, payload types include primitive types, C string, and binary.
//!    The payload can also include one or multiple other attributes,
//!    known as nested attributes.
//!
//! Similar to [`super::segment::NlSegment`], attributes have alignment requirements;
//! both the header and payload must be aligned to [`super::NLMSG_ALIGN`]
//! when being transferred to and from user space.
//!
//! The layout of a netlink attribute is depicted as follows:
//!
//! ┌────────┬─────────┬─────────┬─────────┐
//! │ Header │ Padding │ Payload │ Padding │
//! └────────┴─────────┴─────────┴─────────┘
//!
//! [`segment`]: super::segment

use align_ext::AlignExt;

use super::NLMSG_ALIGN;
use crate::{
    prelude::*,
    util::{MultiRead, MultiWrite},
};

pub mod noattr;

/// Netlink attribute header.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/netlink.h#L229>.
//
// The layout of the `type_` field is structured as follows:
// ┌────────┬───────────────┬──────────┐
// │ Nested │ Net Byteorder │ Payload  │
// └────────┴───────────────┴──────────┘
//   bit 15      bit 14       bits 13-0
#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct CAttrHeader {
    len: u16,
    type_: u16,
}

impl CAttrHeader {
    pub fn type_(&self) -> u16 {
        self.type_ & ATTRIBUTE_TYPE_MASK
    }
}

const IS_NESTED_MASK: u16 = 1u16 << 15;
const IS_NET_BYTEORDER_MASK: u16 = 1u16 << 14;
const ATTRIBUTE_TYPE_MASK: u16 = !(IS_NESTED_MASK | IS_NET_BYTEORDER_MASK);

/// Netlink Attribute.
pub trait Attribute: Debug + Send + Sync {
    /// Returns the type of the attribute.
    fn type_(&self) -> u16;

    /// Returns the byte representation of the payload.
    fn payload_as_bytes(&self) -> &[u8];

    /// Returns the payload length (excluding padding).
    fn payload_len(&self) -> usize {
        self.payload_as_bytes().len()
    }

    /// Returns the total length of the attribute (header + payload, excluding padding).
    fn total_len(&self) -> usize {
        core::mem::size_of::<CAttrHeader>() + self.payload_len()
    }

    /// Returns the total length of the attribute (header + payload, including padding).
    fn total_len_with_padding(&self) -> usize {
        self.total_len().align_up(NLMSG_ALIGN)
    }

    /// Returns the length of the padding bytes.
    fn padding_len(&self) -> usize {
        self.total_len_with_padding() - self.total_len()
    }

    /// Reads the attribute from the `reader`.
    fn read_from(reader: &mut dyn MultiRead) -> Result<Self>
    where
        Self: Sized;

    /// Reads all attributes from the reader.
    ///
    /// The cumulative length of the read attributes must not exceed total_len.
    fn read_all_from(reader: &mut dyn MultiRead, mut total_len: usize) -> Result<Vec<Self>>
    where
        Self: Sized,
    {
        let mut res = Vec::new();

        while total_len > 0 {
            let attr = Self::read_from(reader)?;
            total_len -= attr.total_len();

            let padding_len = attr.padding_len().min(total_len);
            reader.skip(padding_len);
            total_len -= padding_len;

            res.push(attr);
        }

        Ok(res)
    }

    /// Writes the attribute to the `writer`.
    fn write_to(&self, writer: &mut dyn MultiWrite) -> Result<()> {
        let header = CAttrHeader {
            type_: self.type_(),
            len: self.total_len() as u16,
        };

        writer.write_val(&header)?;
        writer.write(&mut VmReader::from(self.payload_as_bytes()))?;

        let padding_len = self.padding_len();
        writer.skip(padding_len);

        Ok(())
    }
}
