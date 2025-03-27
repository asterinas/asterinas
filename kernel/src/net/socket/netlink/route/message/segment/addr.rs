// SPDX-License-Identifier: MPL-2.0

use core::num::NonZeroU32;

use align_ext::AlignExt;

use super::{header::CMessageSegmentHeader, legacy::CRtGenMsg, NlSegmentCommonOps, SegmentBody};
use crate::{
    net::socket::netlink::route::message::{
        attr::{addr::NlAddrAttr, NlAttr},
        util::{align_reader, align_writer},
        NLMSG_ALIGN,
    },
    prelude::*,
    util::net::CSocketAddrFamily,
};

#[derive(Debug, Getters)]
#[getset(get = "pub")]
pub struct AddrSegment {
    header: CMessageSegmentHeader,
    body: AddrSegmentBody,
    attrs: Vec<NlAddrAttr>,
}

impl AddrSegment {
    pub fn new(
        header: CMessageSegmentHeader,
        body: AddrSegmentBody,
        attrs: Vec<NlAddrAttr>,
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

impl NlSegmentCommonOps for AddrSegment {
    const BODY_LEN: usize = size_of::<CIfaddrMsg>();

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

    fn read_from(header: CMessageSegmentHeader, reader: &mut VmReader) -> Result<Self>
    where
        Self: Sized,
    {
        let (body, body_len) = AddrSegmentBody::read_body_from_user(&header, reader)?;

        let attrs = {
            let attrs_len = (header.len as usize - size_of::<CMessageSegmentHeader>() - body_len)
                .align_down(NLMSG_ALIGN);
            if attrs_len > 0 {
                align_reader(reader)?;
            }
            NlAddrAttr::read_all_from(reader, attrs_len)?
        };

        Ok(Self {
            header,
            body,
            attrs,
        })
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
}

impl SegmentBody for AddrSegmentBody {
    type LegacyType = CRtGenMsg;
    type CType = CIfaddrMsg;

    // FIXME: Further check whether we need validate the value.
}

/// Corresponding to `ifaddrmsg` in Linux
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct CIfaddrMsg {
    pub family: u8,
    /// The prefix length
    pub prefix_len: u8,
    /// Flags
    pub flags: u8,
    /// Address scope
    pub scope: u8,
    /// Link index
    pub index: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct AddrSegmentBody {
    pub family: CSocketAddrFamily,
    pub prefix_len: u8,
    pub flags: AddrMessageFlags,
    pub scope: RtScope,
    pub index: Option<NonZeroU32>,
}

impl TryFrom<CIfaddrMsg> for AddrSegmentBody {
    type Error = Error;

    fn try_from(value: CIfaddrMsg) -> Result<Self> {
        let family = CSocketAddrFamily::try_from(value.family as i32)?;
        // TODO: If the attribute IFA_FLAGS exists, the flags in header should be ignored.
        let flags = AddrMessageFlags::from_bits_truncate(value.flags as u32);
        let scope = RtScope::try_from(value.scope)?;
        let index = NonZeroU32::new(value.index);

        Ok(Self {
            family,
            prefix_len: value.prefix_len,
            flags,
            scope,
            index,
        })
    }
}

impl From<AddrSegmentBody> for CIfaddrMsg {
    fn from(value: AddrSegmentBody) -> Self {
        let index = if let Some(index) = value.index {
            index.get()
        } else {
            0
        };
        CIfaddrMsg {
            family: value.family as u8,
            prefix_len: value.prefix_len,
            flags: value.flags.bits() as u8,
            scope: value.scope as _,
            index,
        }
    }
}

bitflags! {
    pub struct AddrMessageFlags: u32 {
        const SECONDARY     = 0x01;
        const NODAD         = 0x02;
        const OPTIMISTIC    = 0x04;
        const DADFAILED     = 0x08;
        const HOMEADDRESS   =0x10;
        const DEPRECATED	=0x20;
        const TENTATIVE		=0x40;
        const PERMANENT		=0x80;
        const MANAGETEMPADDR=0x100;
        const NOPREFIXROUTE	=0x200;
        const MCAUTOJOIN	=0x400;
        const STABLE_PRIVACY=0x800;
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, TryFromInt)]
pub enum RtScope {
    UNIVERSE = 0,
    // User defined values
    SITE = 200,
    LINK = 253,
    HOST = 254,
    NOWHERE = 255,
}
