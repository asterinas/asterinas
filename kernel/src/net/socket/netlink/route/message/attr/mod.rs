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

use super::NLMSG_ALIGN;
use crate::{
    prelude::*,
    util::{MultiRead, MultiWrite},
};

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

    /// Writes the attribute to user space.
    ///
    /// If this operation returns success, the function will returns the actual write len.
    fn write_attr_to_user(&self, writer: &mut dyn MultiWrite) -> Result<()> {
        let header = CNlAttrHeader {
            type_: self.type_(),
            len: self.total_len() as u16,
        };

        writer.align_to(NLMSG_ALIGN);
        writer.write_val(&header)?;
        writer.write(&mut VmReader::from(self.payload_as_bytes()))?;

        Ok(())
    }

    fn as_any(&self) -> &dyn Any;
}

pub trait ReadAttrFromUser: Sized + NlAttr {
    type Payload;

    fn new(payload: Self::Payload) -> Self;
    fn read_payload_from_user(reader: &mut dyn MultiRead, len: usize) -> Result<Self::Payload>;
    fn read_from_user(reader: &mut dyn MultiRead, header: &CNlAttrHeader) -> Result<Self> {
        let payload = {
            let len = header.len as usize - core::mem::size_of_val(header);
            Self::read_payload_from_user(reader, len)?
        };

        let res = Self::new(payload);

        Ok(res)
    }
}

macro_rules! define_attribute {
    // First pattern: The payload is CString
    ($attr_enum:expr, $attr_name:ident, CString, $max_len:expr) => {
        #[derive(Debug)]
        pub struct $attr_name {
            pub value: CString,
        }

        impl NlAttr for $attr_name {
            fn type_(&self) -> u16 {
                $attr_enum as u16
            }

            fn payload_as_bytes(&self) -> &[u8] {
                self.value.as_bytes_with_nul()
            }

            fn as_any(&self) -> &dyn Any {
                self
            }
        }

        impl ReadAttrFromUser for $attr_name {
            type Payload = CString;

            fn new(value: Self::Payload) -> Self {
                Self { value }
            }

            fn read_payload_from_user(
                reader: &mut dyn MultiRead,
                len: usize,
            ) -> Result<Self::Payload> {
                let max_len = $max_len.min(len);
                reader.read_cstring_with_max_len(max_len)
            }
        }
    };
    // Second pattern: The payload is primitive, i.e., any type that implements Pod trait.
    ($attr_enum:expr, $attr_name:ident, $payload_type: ty) => {
        #[derive(Debug)]
        pub struct $attr_name {
            pub value: $payload_type,
        }

        impl NlAttr for $attr_name {
            fn payload_as_bytes(&self) -> &[u8] {
                self.value.as_bytes()
            }

            fn type_(&self) -> u16 {
                $attr_enum as u16
            }

            fn as_any(&self) -> &dyn Any {
                self
            }
        }

        impl ReadAttrFromUser for $attr_name {
            type Payload = $payload_type;

            fn new(value: $payload_type) -> Self {
                Self { value }
            }

            fn read_payload_from_user(
                reader: &mut dyn MultiRead,
                len: usize,
            ) -> Result<Self::Payload> {
                if len != core::mem::size_of::<$payload_type>() {
                    return_errno_with_message!(Errno::EINVAL, "invalid length");
                }

                reader.read_val::<$payload_type>()
            }
        }
    }; // TODO: The payload is nested attributes
}

macro_rules! read_attrs_util {
    ($attrs_len: expr, $reader: expr, $attr_class: ty, ($($attr_enum: pat => $attr_ty: ty),*)) => {{
        let mut res = Vec::new();

        while $attrs_len > 0 {
            let align_offset = $reader.align_to(NLMSG_ALIGN);
            $attrs_len -= align_offset;

            let header = $reader.read_val::<CNlAttrHeader>()?;

            match <$attr_class>::try_from(header.type_())? {
                $(
                    $attr_enum => {
                        let attr = Box::new(<$attr_ty>::read_from_user($reader, &header)?) as Box<dyn NlAttr>;
                        $attrs_len -= attr.total_len_with_padding();
                        res.push(attr);
                    }
                ),*
                _ => todo!("parse other attributes"),
            }
        }

        Ok(res)
    }}
}

/// The size limit of interface name
const IFNAME_SIZE: usize = 16;

use define_attribute;
use read_attrs_util;
