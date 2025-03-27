// SPDX-License-Identifier: MPL-2.0

use core::num::NonZeroU32;

use align_ext::AlignExt;
use aster_bigtcp::iface::{InterfaceFlags, InterfaceType};

use super::{header::CMessageSegmentHeader, legacy::CRtGenMsg, NlSegmentCommonOps, SegmentBody};
use crate::{
    net::socket::netlink::route::message::{
        attr::{link::NlLinkAttr, NlAttr},
        util::{align_reader, align_writer},
        NLMSG_ALIGN,
    },
    prelude::*,
    util::net::CSocketAddrFamily,
};

#[derive(Debug, Getters)]
#[getset(get = "pub")]
pub struct LinkSegment {
    header: CMessageSegmentHeader,
    body: LinkSegmentBody,
    attrs: Vec<NlLinkAttr>,
}

impl LinkSegment {
    pub fn new(
        header: CMessageSegmentHeader,
        body: LinkSegmentBody,
        attrs: Vec<NlLinkAttr>,
    ) -> Self {
        let mut segment = Self {
            header,
            body,
            attrs,
        };

        segment.header.len = segment.total_len() as u32;

        segment
    }
}

impl NlSegmentCommonOps for LinkSegment {
    const BODY_LEN: usize = size_of::<CIfinfoMsg>();

    fn header(&self) -> &CMessageSegmentHeader {
        &self.header
    }

    fn header_mut(&mut self) -> &mut CMessageSegmentHeader {
        &mut self.header
    }

    fn attrs_len(&self) -> usize {
        self.attrs
            .iter()
            .map(|attr| attr.total_len_with_padding())
            .sum()
    }

    fn write_to(&self, writer: &mut VmWriter) -> Result<()> {
        align_writer(writer)?;
        writer.write_val(&self.header)?;
        self.body.write_body_to_user(writer)?;
        for attr in self.attrs.iter() {
            attr.write_to(writer)?;
        }

        Ok(())
    }

    fn read_from(header: CMessageSegmentHeader, reader: &mut VmReader) -> Result<Self>
    where
        Self: Sized,
    {
        let (body, body_len) = LinkSegmentBody::read_body_from_user(&header, reader)?;

        let attrs = {
            let attrs_len = (header.len as usize - size_of::<CMessageSegmentHeader>() - body_len)
                .align_down(NLMSG_ALIGN);
            if attrs_len > 0 {
                align_reader(reader)?;
            }
            NlLinkAttr::read_all_from(reader, attrs_len)?
        };

        Ok(Self {
            header,
            body,
            attrs,
        })
    }
}

impl SegmentBody for LinkSegmentBody {
    type LegacyType = CRtGenMsg;
    type CType = CIfinfoMsg;
}

/// Link level specific information, corresponding to `ifinfomsg` in Linux
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct CIfinfoMsg {
    /// AF_UNSPEC
    pub family: u8,
    /// Padding byte
    pub _pad: u8,
    /// Device type
    pub type_: u16,
    /// Interface index
    pub index: u32,
    /// Device flags
    pub flags: u32,
    /// Change mask
    pub change: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct LinkSegmentBody {
    pub family: CSocketAddrFamily,
    pub type_: InterfaceType,
    pub index: Option<NonZeroU32>,
    pub flags: InterfaceFlags,
}

impl TryFrom<CIfinfoMsg> for LinkSegmentBody {
    type Error = Error;

    fn try_from(value: CIfinfoMsg) -> Result<Self> {
        if value.change != 0 || value._pad != 0 {
            return_errno_with_message!(Errno::EINVAL, "`change` and `_pad` field must be zero");
        }

        let family = CSocketAddrFamily::try_from(value.family as i32)?;
        let type_ = InterfaceType::try_from(value.type_)?;
        let index = NonZeroU32::new(value.index);
        let flags = InterfaceFlags::from_bits_truncate(value.flags);

        Ok(Self {
            family,
            type_,
            index,
            flags,
        })
    }
}

impl From<LinkSegmentBody> for CIfinfoMsg {
    fn from(value: LinkSegmentBody) -> Self {
        CIfinfoMsg {
            family: value.family as _,
            _pad: 0,
            type_: value.type_ as _,
            index: value.index.map(NonZeroU32::get).unwrap_or(0),
            flags: value.flags.bits(),
            change: 0,
        }
    }
}
