// SPDX-License-Identifier: MPL-2.0

//! The netlink message types for netlink route protocol.

mod addr;
mod attributes;
mod legacy;
mod link;
mod overview;
mod route;
mod util;

pub use addr::{AddrMessage, AddrMessageFlags, AddrSegment, CAddrMessage, RtScope};
pub use attributes::{
    AttrOps, CNetlinkAttrHeader, IfName, IfaAddress, IfaLable, IfaLocal, LinkAttrType,
    ReadAttrFromUser, TxqLen,
};
pub use legacy::CRtGenMessage;
pub use link::{CLinkMessage, LinkMessage, LinkSegment};
use ostd::early_println;
pub use overview::*;
pub use util::{AckMessage, NetDeviceFlags, NetDeviceType};

use crate::{
    net::socket::netlink::message::{
        CNetlinkMessageHeader, GetRequestFlags, NetlinkMessageCommonFlags,
    },
    prelude::*,
    util::MultiWrite,
};

#[repr(u16)]
#[derive(Debug, Clone, Copy, TryFromInt, PartialEq, Eq, PartialOrd, Ord)]
pub enum CMessageType {
    // Standard netlink message types
    NOOP = 1,
    ERROR = 2,
    DONE = 3,
    OVERRUN = 4,

    // protocol-level types
    NEWLINK = 16,
    DELLINK = 17,
    GETLINK = 18,
    SETLINK = 19,

    NEWADDR = 20,
    DELADDR = 21,
    GETADDR = 22,

    NEWROUTE = 24,
    DELROUTE = 25,
    GETROUTE = 26,
    // TODO: The list is not exhaustive now.
}

impl CMessageType {
    const fn is_new_ruquest(&self) -> bool {
        (*self as u16) & 0x3 == 0x0
    }

    const fn is_del_request(&self) -> bool {
        (*self as u16) & 0x3 == 0x1
    }

    const fn is_get_request(&self) -> bool {
        (*self as u16) & 0x3 == 0x2
    }
}

pub trait AnyRequestMessage: Send + Sync + Any + Debug {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

pub trait AnyResponseMessage: Send + Sync + Any {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

#[derive(Debug)]
pub struct GetRequest<M: Debug> {
    // Header
    header: CNetlinkMessageHeader,
    // Content
    message: M,
}

impl<M: Debug> GetRequest<M> {
    pub fn new(header: CNetlinkMessageHeader, message: M) -> Self {
        let common_flags = NetlinkMessageCommonFlags::from_bits_truncate(header.flags);
        debug_assert_eq!(common_flags, NetlinkMessageCommonFlags::REQUEST);

        Self { header, message }
    }

    pub const fn header(&self) -> &CNetlinkMessageHeader {
        &self.header
    }

    pub const fn message(&self) -> &M {
        &self.message
    }

    pub const fn flags(&self) -> GetRequestFlags {
        GetRequestFlags::from_bits_truncate(self.header.flags)
    }
}

pub enum ResponseContent {
    Link(Vec<LinkMessage>, Option<AckMessage>),
    Addr(Vec<AddrMessage>, Option<AckMessage>),
}

pub struct GetResponse {
    // Header
    request_header: CNetlinkMessageHeader,

    // Response
    content: ResponseContent,
}

impl GetResponse {
    pub const fn new(request_header: &CNetlinkMessageHeader, content: ResponseContent) -> Self {
        Self {
            request_header: *request_header,
            content,
        }
    }

    pub fn write_to_user(&self, writer: &mut dyn MultiWrite) -> Result<usize> {
        match &self.content {
            ResponseContent::Link(links, ack) => {
                write_link_response_to_user(links, ack, &self.request_header, writer)
            }
            ResponseContent::Addr(addrs, ack) => {
                write_addr_response_to_user(addrs, ack, &self.request_header, writer)
            }
        }
    }
}

fn write_link_response_to_user(
    links: &Vec<LinkMessage>,
    ack: &Option<AckMessage>,
    request_header: &CNetlinkMessageHeader,
    writer: &mut dyn MultiWrite,
) -> Result<usize> {
    let header_flags = {
        let ack_len = if ack.is_some() { 1 } else { 0 };

        if links.len() + ack_len > 1 {
            // If multiple parts, the ack is essential
            debug_assert!(ack.is_some());
            NetlinkMessageCommonFlags::MULTI
        } else {
            NetlinkMessageCommonFlags::empty()
        }
    };

    let mut written_len = 0;
    for link in links {
        // 1. Write header
        let header = {
            let len = {
                // let attribute_len: usize = link
                //     .attrs
                //     .iter()
                //     .map(|attr| attr.total_len_with_padding())
                //     .sum();
                let attribute_len = 0;
                core::mem::size_of::<CNetlinkMessageHeader>()
                    + core::mem::size_of::<CLinkMessage>()
                    + attribute_len
            };

            CNetlinkMessageHeader {
                len: len as u32,
                type_: CMessageType::NEWLINK as _,
                flags: header_flags.bits(),
                seq: request_header.seq,
                pid: request_header.pid,
            }
        };

        writer.write_val(&header)?;
        written_len += core::mem::size_of_val(&header);

        // 2. Write link message
        let c_link_message = link.to_c();
        writer.write_val(&c_link_message)?;
        written_len += core::mem::size_of_val(&c_link_message);

        // 3. Write attributes
        // for attr in link.attrs.iter() {
        //     let attr_len = attr.write_to_user(writer)?;
        //     written_len += attr_len;
        // }
    }

    if let Some(ack_message) = ack {
        let ack_len = ack_message.write_to_user(writer)?;
        written_len += ack_len;
    }

    Ok(written_len)
}

fn write_addr_response_to_user(
    addrs: &Vec<AddrMessage>,
    ack: &Option<AckMessage>,
    request_header: &CNetlinkMessageHeader,
    writer: &mut dyn MultiWrite,
) -> Result<usize> {
    early_println!("write address response");

    let header_flags = {
        let ack_len = if ack.is_some() { 1 } else { 0 };

        if addrs.len() + ack_len > 1 {
            // If multiple parts, the ack is essential
            debug_assert!(ack.is_some());
            NetlinkMessageCommonFlags::MULTI
        } else {
            NetlinkMessageCommonFlags::empty()
        }
    };

    let mut written_len = 0;
    for addr in addrs {
        println!("addr: {:?}\n", addr);
        // 1. Write header
        let header = {
            let len = {
                // let attribute_len: usize = addr
                //     .attrs
                //     .iter()
                //     .map(|attr| attr.total_len_with_padding())
                //     .sum();
                let attribute_len = 0;
                core::mem::size_of::<CNetlinkMessageHeader>()
                    + core::mem::size_of::<CAddrMessage>()
                    + attribute_len
            };

            CNetlinkMessageHeader {
                len: len as u32,
                type_: CMessageType::NEWADDR as _,
                flags: header_flags.bits(),
                seq: request_header.seq,
                pid: request_header.pid,
            }
        };

        writer.write_val(&header)?;
        written_len += core::mem::size_of_val(&header);

        // 2. Write addr message
        let c_addr_message = addr.to_c();
        writer.write_val(&c_addr_message)?;
        written_len += core::mem::size_of_val(&c_addr_message);

        // 3. Write attributes
        // for attr in addr.attrs.iter() {
        //     println!("write attr = {:?}", attr);
        //     let attr_len = attr.write_to_user(writer)?;
        //     written_len += attr_len;
        // }
    }

    if let Some(ack_message) = ack {
        let ack_len = ack_message.write_to_user(writer)?;
        written_len += ack_len;
    }

    Ok(written_len)
}

macro_rules! impl_any_request_message {
    ($message_type:ty) => {
        impl AnyRequestMessage for $message_type {
            fn as_any(&self) -> &dyn Any {
                self
            }

            fn as_any_mut(&mut self) -> &mut dyn Any {
                self
            }
        }
    };
}

macro_rules! impl_any_response_message {
    ($message_type:ty) => {
        impl AnyResponseMessage for $message_type {
            fn as_any(&self) -> &dyn Any {
                self
            }

            fn as_any_mut(&mut self) -> &mut dyn Any {
                self
            }
        }
    };
}

impl_any_request_message!(GetRequest<LinkMessage>);
impl_any_request_message!(GetRequest<AddrMessage>);
impl_any_response_message!(GetResponse);
