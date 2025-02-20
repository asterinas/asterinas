// SPDX-License-Identifier: MPL-2.0

use core::num::NonZeroU32;

use super::{AttrOps, CRtGenMessage, NlMsgSegment, ReadBodyFromUser, ReadNlMsgSegmentFromUser};
use crate::{
    net::socket::netlink::message::CNetlinkMessageHeader,
    prelude::*,
    util::{net::CSocketAddrFamily, MultiRead},
};

#[derive(Debug, Getters)]
#[getset(get = "pub")]
pub struct AddrSegment {
    header: CNetlinkMessageHeader,
    body: AddrMessage,
    attrs: Vec<Box<dyn AttrOps>>,
}

impl NlMsgSegment for AddrSegment {
    fn header(&self) -> &CNetlinkMessageHeader {
        &self.header
    }

    fn header_mut(&mut self) -> &mut CNetlinkMessageHeader {
        &mut self.header
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn body_len(&self) -> usize {
        size_of::<CAddrMessage>()
    }

    fn attrs(&self) -> &Vec<Box<dyn AttrOps>> {
        &self.attrs
    }
}

impl ReadNlMsgSegmentFromUser for AddrSegment {
    type Body = AddrMessage;

    fn new(header: CNetlinkMessageHeader, body: Self::Body, attrs: Vec<Box<dyn AttrOps>>) -> Self {
        let mut segment = Self {
            header,
            body,
            attrs,
        };
        segment.adjust_header_len();
        segment
    }

    fn read_attrs(attrs_len: usize, _reader: &mut dyn MultiRead) -> Result<Vec<Box<dyn AttrOps>>> {
        while attrs_len > 0 {
            todo!()
        }
        Ok(Vec::new())
    }
}

impl ReadBodyFromUser for AddrMessage {
    type LegacyType = CRtGenMessage;
    type CType = CAddrMessage;
}

/// Corresponding to `ifaddrmsg` in Linux
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct CAddrMessage {
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

#[derive(Debug)]
pub struct AddrMessage {
    pub family: CSocketAddrFamily,
    pub prefix_len: u8,
    pub flags: AddrMessageFlags,
    pub scope: RtScope,
    pub index: Option<NonZeroU32>,
}

impl TryFrom<CAddrMessage> for AddrMessage {
    type Error = Error;

    fn try_from(value: CAddrMessage) -> Result<Self> {
        let family = CSocketAddrFamily::try_from(value.family as i32)?;
        // If the attribute IFA_FLAGS exists, the flags in header should be ignored.
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

impl AddrMessage {
    pub fn to_c(&self) -> CAddrMessage {
        let index = if let Some(index) = self.index {
            index.get()
        } else {
            0
        };
        CAddrMessage {
            family: self.family as u8,
            prefix_len: self.prefix_len,
            flags: self.flags.bits() as u8,
            scope: self.scope as _,
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
