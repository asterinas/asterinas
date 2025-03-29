// SPDX-License-Identifier: MPL-2.0

//! Netlink attributes.
//!
//! Netlink attributes provide additional information for each [`segment`].
//! Each netlink attribute consists of two parts:
//! 1. Header. The attribute header is of type [`CNetlinkAttrHeader`],
//! which defines the type and length of the attribute. Note that the attribute
//! type can belong to different classes, determined by the segment type.
//! 2. Payload. The payload of the attribute, which may vary in type.
//! Currently, payload types include primitive types, CString, and binary.
//! The payload can also consist of one or multiple other attributes,
//! known as nested attributes.
//!
//! Similar to [`super::NlMsgSegment`], the attribute also has alignment requirements;
//! both header and payload must be aligned to [`super::NLMSG_ALIGN`]
//! when copying to and from user space.
//!
//! The layout of a netlink attribute is shown as follows:
//!
//! ┌────────┬─────────┬─────────┬─────────┐
//! │ Header │ Padding │ Payload │ Padding │
//! └────────┴─────────┴─────────┴─────────┘
//!
//! [`segment`]: super::segment

use align_ext::AlignExt;

use super::{
    util::{align_reader, align_writer},
    NLMSG_ALIGN,
};
use crate::{prelude::*, util::MultiWrite};

pub(in crate::net::socket::netlink) mod addr;
pub(in crate::net::socket::netlink) mod link;

/// Netlink attribute header.
//
// The layout of the `type_` field is as follows:
// ┌────────┬───────────────┬──────────┐
// │ Nested │ Net Byteorder │ Payload  │
// └────────┴───────────────┴──────────┘
//   bit 15      bit 14       bits 13-0
#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct CNlAttrHeader {
    len: u16,
    type_: u16,
}

impl CNlAttrHeader {
    pub fn type_(&self) -> u16 {
        self.type_ & ATTRIBUTE_TYPE_MASK
    }
}

const IS_NESTED_MASK: u16 = 1u16 << 15;
const IS_NET_BYTEORDER_MASK: u16 = 1u16 << 14;
const ATTRIBUTE_TYPE_MASK: u16 = !(IS_NESTED_MASK | IS_NET_BYTEORDER_MASK);

/// Netlink Attribute
pub trait NlAttr: Debug + Send + Sync {
    /// Returns the type of the attribute
    fn type_(&self) -> u16;

    /// Returns the bytes representabtion of the payload
    fn payload_as_bytes(&self) -> &[u8];

    /// Returns the attribute payload len(w/o padding)
    fn payload_len(&self) -> usize {
        self.payload_as_bytes().len()
    }

    /// Returns the total len of the attribute(header + payload, w/o padding)
    fn total_len(&self) -> usize {
        core::mem::size_of::<CNlAttrHeader>() + self.payload_len()
    }

    /// Returns the total len of the attribute(header + payload, w/ padding)
    fn total_len_with_padding(&self) -> usize {
        self.total_len().align_up(NLMSG_ALIGN)
    }

    fn read_from(reader: &mut VmReader) -> Result<Self>
    where
        Self: Sized;

    fn read_all_from(reader: &mut VmReader, mut total_len: usize) -> Result<Vec<Self>>
    where
        Self: Sized,
    {
        let mut res = Vec::new();

        while total_len > 0 {
            let align_offset = align_reader(reader)?;
            total_len -= align_offset;

            if total_len == 0 {
                break;
            }

            let attr = Self::read_from(reader)?;
            total_len -= attr.total_len();
            res.push(attr);
        }

        Ok(res)
    }

    /// Writes the attribute to user space.
    fn write_to(&self, writer: &mut VmWriter) -> Result<()> {
        let header = CNlAttrHeader {
            type_: self.type_(),
            len: self.total_len() as u16,
        };

        align_writer(writer)?;
        writer.write_val(&header)?;
        writer.write(&mut VmReader::from(self.payload_as_bytes()))?;

        Ok(())
    }
}

/// The size limit of interface name
const IFNAME_SIZE: usize = 16;
