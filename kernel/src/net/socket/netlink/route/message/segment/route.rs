// SPDX-License-Identifier: MPL-2.0

use super::legacy::CRtGenMsg;
use crate::{
    net::{
        route::{RouteProtocol, RouteScope, RouteTableId, RouteType},
        socket::netlink::{
            message::{SegmentBody, SegmentCommon},
            route::message::attr::route::RouteAttr,
        },
    },
    prelude::*,
    util::net::CSocketAddrFamily,
};

pub type RouteSegment = SegmentCommon<RouteSegmentBody, RouteAttr>;

bitflags! {
    /// Route message flags.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/rtnetlink.h#L338-L352>.
    pub struct RouteFlags: u32 {
        /// Requests notifications for route changes.
        const NOTIFY = 0x100;
        /// Identifies cloned or cached lookup results.
        const CLONED = 0x200;
        /// Identifies equal-cost multipath balancing.
        const EQUALIZE = 0x400;
        /// Identifies a prefix route.
        const PREFIX = 0x800;
        /// Requests the lookup table to be reported.
        const LOOKUP_TABLE = 0x1000;
        /// Requests the matched FIB route instead of a cloned lookup result.
        const FIB_MATCH = 0x2000;
    }
}

impl SegmentBody for RouteSegmentBody {
    type CLegacyType = CRtGenMsg;
    type CType = CRtMsg;
}

/// `rtmsg` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/rtnetlink.h#L237>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct CRtMsg {
    pub family: u8,
    pub dst_len: u8,
    pub src_len: u8,
    pub tos: u8,
    pub table: u8,
    pub protocol: u8,
    pub scope: u8,
    pub type_: u8,
    pub flags: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct RouteSegmentBody {
    pub family: i32,
    pub dst_len: u8,
    pub src_len: u8,
    pub tos: u8,
    pub table: Option<RouteTableId>,
    pub protocol: RouteProtocol,
    pub scope: RouteScope,
    pub type_: RouteType,
    pub flags: RouteFlags,
}

impl TryFrom<CRtMsg> for RouteSegmentBody {
    type Error = Error;

    fn try_from(value: CRtMsg) -> Result<Self> {
        let family = value.family as i32;
        let max_prefix_len = match family {
            family if family == CSocketAddrFamily::AF_INET as i32 => 32,
            family if family == CSocketAddrFamily::AF_INET6 as i32 => 128,
            family if family == CSocketAddrFamily::AF_UNSPEC as i32 => 128,
            _ => return_errno_with_message!(Errno::EAFNOSUPPORT, "the route family is unsupported"),
        };
        if value.dst_len > max_prefix_len || value.src_len > max_prefix_len {
            return_errno_with_message!(Errno::EINVAL, "the route prefix length is invalid");
        }
        Ok(Self {
            family,
            dst_len: value.dst_len,
            src_len: value.src_len,
            tos: value.tos,
            table: table_from_rtmsg(value.table),
            protocol: RouteProtocol::new(value.protocol),
            scope: RouteScope::new(value.scope),
            type_: RouteType::try_from(value.type_)?,
            flags: RouteFlags::from_bits(value.flags).ok_or_else(|| {
                Error::with_message(Errno::EOPNOTSUPP, "the route flags are not supported")
            })?,
        })
    }
}

impl From<RouteSegmentBody> for CRtMsg {
    fn from(value: RouteSegmentBody) -> Self {
        Self {
            family: value.family as u8,
            dst_len: value.dst_len,
            src_len: value.src_len,
            tos: value.tos,
            table: value.table.map(table_to_rtmsg).unwrap_or(0),
            protocol: value.protocol.get(),
            scope: value.scope.get(),
            type_: value.type_ as u8,
            flags: value.flags.bits(),
        }
    }
}

impl From<CRtGenMsg> for CRtMsg {
    fn from(value: CRtGenMsg) -> Self {
        Self {
            family: value.family,
            dst_len: 0,
            src_len: 0,
            tos: 0,
            table: 0,
            protocol: 0,
            scope: 0,
            type_: RouteType::Unspec as u8,
            flags: 0,
        }
    }
}

fn table_from_rtmsg(table: u8) -> Option<RouteTableId> {
    if table == RouteTableId::UNSPEC.get() as u8 {
        None
    } else {
        Some(RouteTableId::new(table as u32))
    }
}

fn table_to_rtmsg(table: RouteTableId) -> u8 {
    if table.get() <= u8::MAX as u32 {
        table.get() as u8
    } else {
        RouteTableId::UNSPEC.get() as u8
    }
}
