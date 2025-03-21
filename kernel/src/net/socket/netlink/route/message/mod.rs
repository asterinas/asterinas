// SPDX-License-Identifier: MPL-2.0

//! The netlink message types for netlink route protocol.
//!
//! This module defines how to interpret the message passed from user space and how to write
//! kernel message to user space.

pub(super) mod attr;
mod segment;
mod util;

pub use attr::{NlAttr, ReadAttrFromUser};
pub(super) use segment::{
    ack::{DoneSegment, ErrorSegment},
    addr::{AddrMessageFlags, AddrSegment, AddrSegmentBody, RtScope},
    header::{CMessageSegmentHeader, GetRequestFlags, SegmentHeaderCommonFlags},
    link::{LinkSegment, LinkSegmentBody},
    CSegmentType, NlMsgSegment, ReadNlMsgSegmentFromUser,
};
pub use util::{NetDeviceFlags, NetDeviceType};

use crate::{
    prelude::*,
    util::{MultiRead, MultiWrite},
};

/// A netlink message.
///
/// A netlink message can be transmitted to and from user space using a single send/receive syscall.
/// It consists of one or more [`NlMsgSegment`]s.
#[derive(Debug)]
pub struct NlMsg {
    segments: Vec<Box<dyn NlMsgSegment>>,
}

macro_rules! read_msg_util {
    ($reader: expr, ($($segment_pat: pat => $segment_type: ty,)*)) => {{
        let mut segments = Vec::new();

        // FIXME: Does request has only one segment? We need to do further check.
        let header = $reader.read_val::<CMessageSegmentHeader>()?;

        match CSegmentType::try_from(header.type_)? {
            $(
                $segment_pat => segments
                .push(Box::new(<$segment_type>::read_from_user(header, $reader)?)
                    as Box<dyn NlMsgSegment>),
            )*
            _ => todo!("support other segments"),
        }

        segments
    }};
}

impl NlMsg {
    pub const fn new(segments: Vec<Box<dyn NlMsgSegment>>) -> Self {
        Self { segments }
    }

    pub fn segments(&self) -> &[Box<dyn NlMsgSegment>] {
        &self.segments
    }

    pub fn segments_mut(&mut self) -> &mut [Box<dyn NlMsgSegment>] {
        &mut self.segments
    }

    pub fn read_from_user(reader: &mut dyn MultiRead) -> Result<Self> {
        let segments = read_msg_util!(reader, (
            CSegmentType::GETLINK => LinkSegment,
            CSegmentType::GETADDR => AddrSegment,
        ));

        Ok(Self { segments })
    }

    pub fn write_to_user(&self, writer: &mut dyn MultiWrite) -> Result<()> {
        for segment in self.segments.iter() {
            segment.write_to_user(writer)?;
        }

        Ok(())
    }

    pub fn total_len(&self) -> usize {
        self.segments
            .iter()
            .map(|segment| segment.header().len as usize)
            .sum()
    }
}

pub const NLMSG_ALIGN: usize = 4;
