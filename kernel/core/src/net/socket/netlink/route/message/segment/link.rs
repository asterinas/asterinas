// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(netlink)]

use core::num::NonZeroU32;

use aster_bigtcp::iface::{InterfaceFlags, InterfaceType};

use super::legacy::CRtGenMsg;
use crate::{
    net::socket::netlink::{
        message::{SegmentBody, SegmentCommon},
        route::message::attr::link::LinkAttr,
    },
    prelude::*,
    util::net::CSocketAddrFamily,
};

pub(in netlink) type LinkSegment = SegmentCommon<LinkSegmentBody, LinkAttr>;

impl SegmentBody for LinkSegmentBody {
    type CLegacyType = CRtGenMsg;
    type CType = CIfinfoMsg;

    fn validate_strict_request_body(body: &Self::CType) -> Result<()> {
        if body._pad != 0 || body.type_ != 0 || body.flags != 0 || body.change != 0 {
            return_errno_with_message!(
                Errno::EINVAL,
                "the link request header contains nonzero padding, type, flags, or change"
            );
        }
        Ok(())
    }
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct LinkSegmentBody {
    pub family: CSocketAddrFamily,
    pub type_: InterfaceType,
    pub index: Option<NonZeroU32>,
    pub flags: InterfaceFlags,
}

impl From<CIfinfoMsg> for LinkSegmentBody {
    fn from(value: CIfinfoMsg) -> Self {
        // The current GETLINK handler does not use the requested address family,
        // so unknown families can fall back to `AF_UNSPEC`.
        let family = CSocketAddrFamily::try_from(value.family as i32)
            .unwrap_or(CSocketAddrFamily::AF_UNSPEC);
        // Non-strict GETLINK requests ignore the interface type.
        // Strict checking rejects nonzero types before this conversion.
        // `NETROM` represents zero here and does not select a NET/ROM interface.
        let type_ = InterfaceType::try_from(value.type_).unwrap_or(InterfaceType::NETROM);
        let index = NonZeroU32::new(value.index);
        let flags = InterfaceFlags::from_bits_truncate(value.flags);

        Self {
            family,
            type_,
            index,
            flags,
        }
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
