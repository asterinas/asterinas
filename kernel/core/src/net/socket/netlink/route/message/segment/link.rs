// SPDX-License-Identifier: MPL-2.0

use super::legacy::CRtGenMsg;
use crate::{
    net::socket::netlink::{
        message::{SegmentBody, SegmentCommon},
        route::message::attr::link::LinkAttr,
    },
    prelude::*,
};

pub(in crate::net::socket::netlink::route) type LinkSegment = SegmentCommon<CIfinfoMsg, LinkAttr>;

impl SegmentBody for CIfinfoMsg {
    type CLegacyType = CRtGenMsg;
}

/// `ifinfomsg` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/rtnetlink.h#L561>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct CIfinfoMsg {
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
