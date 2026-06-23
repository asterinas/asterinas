// SPDX-License-Identifier: MPL-2.0

use core::num::NonZeroU32;

use super::legacy::CRtGenMsg;
use crate::{
    net::{
        route::RouteScope,
        socket::netlink::{
            message::{SegmentBody, SegmentCommon},
            route::message::attr::addr::AddrAttr,
        },
    },
    prelude::*,
};

pub type AddrSegment = SegmentCommon<AddrSegmentBody, AddrAttr>;

impl SegmentBody for AddrSegmentBody {
    type CLegacyType = CRtGenMsg;
    type CType = CIfaddrMsg;
}

/// `ifaddrmsg` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/if_addr.h#L8>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
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

#[derive(Clone, Copy, Debug)]
pub struct AddrSegmentBody {
    pub family: i32,
    pub prefix_len: u8,
    pub flags: AddrMessageFlags,
    pub scope: RouteScope,
    pub index: Option<NonZeroU32>,
}

impl TryFrom<CIfaddrMsg> for AddrSegmentBody {
    type Error = Error;

    fn try_from(value: CIfaddrMsg) -> Result<Self> {
        // TODO: If the attribute IFA_FLAGS exists, the flags in header should be ignored.
        let flags = AddrMessageFlags::from_bits_truncate(value.flags as u32);
        let scope = RouteScope::new(value.scope);
        let index = NonZeroU32::new(value.index);

        Ok(Self {
            family: value.family as i32,
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
            scope: value.scope.get(),
            index,
        }
    }
}

bitflags! {
    /// Flags in [`CIfaddrMsg`].
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/if_addr.h#L45>.
    pub struct AddrMessageFlags: u32 {
        const SECONDARY      = 0x01;
        const NODAD          = 0x02;
        const OPTIMISTIC     = 0x04;
        const DADFAILED      = 0x08;
        const HOMEADDRESS    = 0x10;
        const DEPRECATED	 = 0x20;
        const TENTATIVE		 = 0x40;
        const PERMANENT		 = 0x80;
        const MANAGETEMPADDR = 0x100;
        const NOPREFIXROUTE	 = 0x200;
        const MCAUTOJOIN	 = 0x400;
        const STABLE_PRIVACY = 0x800;
    }
}
