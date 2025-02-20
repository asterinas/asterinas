// SPDX-License-Identifier: MPL-2.0

use core::num::NonZeroU32;

use super::{
    attributes::AttrOps,
    util::{NetDeviceFlags, NetDeviceType},
    CMessageType, CNetlinkAttrHeader, CRtGenMessage, IfName, LinkAttrType, NlMsgSegment,
    ReadAttrFromUser, ReadBodyFromUser, ReadNlMsgSegmentFromUser,
};
use crate::{
    net::socket::netlink::message::CNetlinkMessageHeader,
    prelude::*,
    util::{net::CSocketAddrFamily, MultiRead},
};

#[derive(Debug, Getters)]
#[getset(get = "pub")]
pub struct LinkSegment {
    header: CNetlinkMessageHeader,
    body: LinkMessage,
    attrs: Vec<Box<dyn AttrOps>>,
}

impl NlMsgSegment for LinkSegment {
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
        size_of::<CLinkMessage>()
    }

    fn attrs(&self) -> &Vec<Box<dyn AttrOps>> {
        &self.attrs
    }
}

impl ReadNlMsgSegmentFromUser for LinkSegment {
    type Body = LinkMessage;

    fn new(header: CNetlinkMessageHeader, body: Self::Body, attrs: Vec<Box<dyn AttrOps>>) -> Self {
        let mut segment = Self {
            header,
            body,
            attrs,
        };
        segment.adjust_header_len();
        segment
    }

    fn read_attrs(
        mut attrs_len: usize,
        reader: &mut dyn MultiRead,
    ) -> Result<Vec<Box<dyn AttrOps>>> {
        let mut res = Vec::new();

        while attrs_len > 0 {
            let header = reader.read_val::<CNetlinkAttrHeader>()?;
            match LinkAttrType::try_from(*header.type_())? {
                LinkAttrType::IFNAME => {
                    let attr =
                        Box::new(IfName::read_from_user(reader, &header)?) as Box<dyn AttrOps>;
                    attrs_len -= attr.total_len_with_padding();
                    res.push(attr);
                }
                _ => todo!("parse other link attr type"),
            }
        }

        Ok(res)
    }
}

impl ReadBodyFromUser for LinkMessage {
    type LegacyType = CRtGenMessage;
    type CType = CLinkMessage;

    fn validate_c_type(header: &CNetlinkMessageHeader, c_type: &Self::CType) -> Result<()> {
        if CMessageType::GETLINK as u16 != header.type_ {
            todo!()
        }

        if c_type._pad != 0 || c_type.type_ != 0 || c_type.flags != 0 || c_type.change != 0 {
            return_errno_with_message!(Errno::EINVAL, "invalid value for getlink")
        }
        Ok(())
    }
}

/// Link level specific information, corresponding to `ifinfomsg` in Linux
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct CLinkMessage {
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

#[derive(Debug)]
pub struct LinkMessage {
    pub family: CSocketAddrFamily,
    pub type_: NetDeviceType,
    pub index: Option<NonZeroU32>,
    pub flags: NetDeviceFlags,
}

impl TryFrom<CLinkMessage> for LinkMessage {
    type Error = Error;

    fn try_from(value: CLinkMessage) -> Result<Self> {
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

impl LinkMessage {
    pub fn to_c(&self) -> CLinkMessage {
        CLinkMessage {
            family: self.family as _,
            _pad: 0,
            type_: self.type_ as _,
            index: self.index.map(NonZeroU32::get).unwrap_or(0),
            flags: self.flags.bits(),
            change: 0,
        }
    }
}
