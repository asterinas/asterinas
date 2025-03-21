// SPDX-License-Identifier: MPL-2.0

use core::num::NonZeroU32;

use super::{
    header::CMessageSegmentHeader, impl_nlsegment_general, legacy::CRtGenMsg, CSegmentType,
    NlMsgSegment, ReadBodyFromUser, ReadNlMsgSegmentFromUser, WriteBodyToUser,
};
use crate::{
    net::socket::netlink::{
        route::message::{attr::link::read_link_attrs, NlAttr, NLMSG_ALIGN},
        NetDeviceFlags, NetDeviceType,
    },
    prelude::*,
    util::{net::CSocketAddrFamily, MultiRead, MultiWrite},
};

#[derive(Debug, Getters)]
#[getset(get = "pub")]
pub struct LinkSegment {
    header: CMessageSegmentHeader,
    body: LinkSegmentBody,
    attrs: Vec<Box<dyn NlAttr>>,
}

impl_nlsegment_general!(LinkSegment, LinkSegmentBody, CIfinfoMsg, read_link_attrs);

impl ReadBodyFromUser for LinkSegmentBody {
    type LegacyType = CRtGenMsg;
    type CType = CIfinfoMsg;

    fn validate_read_value(header: &CMessageSegmentHeader, c_type: &Self::CType) -> Result<()> {
        if CSegmentType::GETLINK as u16 != header.type_ {
            todo!()
        }

        if c_type._pad != 0 || c_type.type_ != 0 || c_type.flags != 0 || c_type.change != 0 {
            return_errno_with_message!(Errno::EINVAL, "invalid value for getlink")
        }

        Ok(())
    }
}

impl WriteBodyToUser for LinkSegmentBody {}

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
    pub type_: NetDeviceType,
    pub index: Option<NonZeroU32>,
    pub flags: NetDeviceFlags,
}

impl TryFrom<CIfinfoMsg> for LinkSegmentBody {
    type Error = Error;

    fn try_from(value: CIfinfoMsg) -> Result<Self> {
        let family = CSocketAddrFamily::try_from(value.family as i32)?;
        let type_ = NetDeviceType::try_from(value.type_)?;
        let index = NonZeroU32::new(value.index);
        let flags = NetDeviceFlags::from_bits_truncate(value.flags);

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
