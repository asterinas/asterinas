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
    /// Creates from the type and the payload length.
    fn from_payload_len(type_: u16, payload_len: usize) -> Self {
        let total_len = payload_len + size_of::<Self>();
        debug_assert!(total_len <= u16::MAX as usize);

        Self {
            len: total_len as u16,
            type_,
        }
    }

    /// Returns the type of the attribute.
    pub fn type_(&self) -> u16 {
        self.type_ & ATTRIBUTE_TYPE_MASK
    }

    /// Returns the payload length (excluding padding).
    pub fn payload_len(&self) -> usize {
        self.len as usize - size_of::<Self>()
    }

    /// Returns the total length of the attribute (header + payload, excluding padding).
    pub fn total_len(&self) -> usize {
        self.len as usize
    }

    /// Returns the total length of the attribute (header + payload, including padding).
    pub fn total_len_with_padding(&self) -> usize {
        (self.len as usize).align_up(NLMSG_ALIGN)
    }

    /// Returns the length of the padding bytes.
    pub fn padding_len(&self) -> usize {
        self.total_len_with_padding() - self.total_len()
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

    /// Returns the total length of the attribute (header + payload, including padding).
    fn total_len_with_padding(&self) -> usize {
        // We don't care the attribute type when calculating the attribute length.
        const DUMMY_TYPE: u16 = 0;

        CAttrHeader::from_payload_len(DUMMY_TYPE, self.payload_as_bytes().len())
            .total_len_with_padding()
    }

    /// Reads the attribute from the `reader`.
    ///
    /// This method may return a `None` if the attribute is not recognized. In that case, however,
    /// it must still skip the payload length (excluding padding), as if the attribute were parsed
    /// properly.
    fn read_from(header: &CAttrHeader, reader: &mut dyn MultiRead) -> Result<Option<Self>>
    where
        Self: Sized;

    /// Reads all attributes from the reader.
    ///
    /// The cumulative length of the read attributes must not exceed `total_len`.
    fn read_all_from(reader: &mut dyn MultiRead, mut total_len: usize) -> Result<Vec<Self>>
    where
        Self: Sized,
    {
        let mut res = Vec::new();

        // Below, we're performing strict validation. Although Linux tends to perform strict
        // validation for new netlink message consumers, it may allow fewer or no validations for
        // legacy consumers. See
        // <https://github.com/torvalds/linux/commit/8cb081746c031fb164089322e2336a0bf5b3070c> for
        // more details.

        while total_len > 0 {
            // Validate the remaining length for the attribute header length.
            if total_len < size_of::<CAttrHeader>() {
                return_errno_with_message!(Errno::EINVAL, "the reader length is too small");
            }

            // Read and validate the attribute header.
            let header = reader.read_val_opt::<CAttrHeader>()?.unwrap();
            if header.total_len() < size_of::<CAttrHeader>() {
                return_errno_with_message!(Errno::EINVAL, "the attribute length is too small");
            }

            // Validate the remaining length for the attribute payload length.
            total_len = total_len.checked_sub(header.total_len()).ok_or_else(|| {
                Error::with_message(Errno::EINVAL, "the reader size is too small")
            })?;

            // Read the attribute.
            if let Some(attr) = Self::read_from(&header, reader)? {
                res.push(attr);
            }

            // Skip the padding bytes.
            let padding_len = total_len.min(header.padding_len());
            reader.skip_some(padding_len);
            total_len -= padding_len;
        }

        Ok(res)
    }

    /// Writes the attribute to the `writer`.
    fn write_to(&self, writer: &mut dyn MultiWrite) -> Result<()> {
        let type_ = self.type_();
        let payload = self.payload_as_bytes();

        let header = CAttrHeader::from_payload_len(type_, payload.len());
        writer.write_val_trunc(&header)?;
        writer.write(&mut VmReader::from(payload))?;

        let padding_len = header.padding_len();
        writer.skip_some(padding_len);

        Ok(())
    }
}
