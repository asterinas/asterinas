// SPDX-License-Identifier: MPL-2.0

use super::legacy::CRtGenMsg;
use crate::{
    net::socket::netlink::{
        message::{SegmentBody, SegmentCommon},
        route::message::attr::addr::AddrAttr,
    },
    prelude::*,
};

pub(in crate::net::socket::netlink::route) type AddrSegment = SegmentCommon<CIfaddrMsg, AddrAttr>;

impl SegmentBody for CIfaddrMsg {
    type CLegacyType = CRtGenMsg;
}

/// `ifaddrmsg` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/if_addr.h#L8>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct CIfaddrMsg {
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

bitflags! {
    /// Flags in [`CIfaddrMsg`].
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/if_addr.h#L45>.
    #[repr(C)]
    #[derive(Pod)]
    pub(crate) struct AddrMessageFlags: u32 {
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

/// `rt_scope_t` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/rtnetlink.h#L320>.
#[expect(clippy::upper_case_acronyms)]
#[repr(u8)]
#[derive(Clone, Copy, Debug, TryFromInt)]
pub(crate) enum RtScope {
    UNIVERSE = 0,
    // User defined values
    SITE = 200,
    LINK = 253,
    HOST = 254,
    NOWHERE = 255,
}
