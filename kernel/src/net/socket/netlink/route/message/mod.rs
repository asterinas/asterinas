// SPDX-License-Identifier: MPL-2.0

//! Netlink message types for the netlink route protocol.
//!
//! This module defines how to interpret messages sent from user space and how to write
//! kernel messages back to user space.

pub(super) mod attr;
mod segment;

pub(super) use segment::{
    ack::{DoneSegment, ErrorSegment},
    addr::{AddrMessageFlags, AddrSegment, AddrSegmentBody, RtScope},
    header::{CMessageSegmentHeader, GetRequestFlags, SegmentHeaderCommonFlags},
    link::{LinkSegment, LinkSegmentBody},
    CSegmentType, MsgSegment,
};

use crate::{
    prelude::*,
    util::{MultiRead, MultiWrite},
};

/// A netlink message.
///
/// A netlink message can be transmitted to and from user space using a single send/receive syscall.
/// It consists of one or more [`NlSegment`]s.
#[derive(Debug)]
pub struct Message {
    segments: Vec<MsgSegment>,
}

impl Message {
    pub const fn new(segments: Vec<MsgSegment>) -> Self {
        Self { segments }
    }

    pub fn segments(&self) -> &[MsgSegment] {
        &self.segments
    }

    pub fn segments_mut(&mut self) -> &mut [MsgSegment] {
        &mut self.segments
    }

    pub fn read_from(reader: &mut dyn MultiRead) -> Result<Self> {
        // FIXME: Does a request contain only one segment? We need to investigate further.
        let segments = {
            let reader = reader.current_reader_mut().ok_or_else(|| Errno::EFAULT)?;
            let segment = MsgSegment::read_from(reader)?;
            vec![segment]
        };

        Ok(Self { segments })
    }

    pub fn write_to(&self, writer: &mut dyn MultiWrite) -> Result<()> {
        for segment in self.segments.iter() {
            let writer = writer.current_writer_mut().ok_or_else(|| Errno::EFAULT)?;
            segment.write_to(writer)?;
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
