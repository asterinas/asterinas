// SPDX-License-Identifier: MPL-2.0

//! The netlink message types for netlink route protocol.
//!
//! This module defines how to interpret the message passed from user space and how to write
//! kernel message to user space.

pub(super) mod attr;
mod segment;
mod util;

pub(super) use segment::{
    ack::{DoneSegment, ErrorSegment},
    addr::{AddrMessageFlags, AddrSegment, AddrSegmentBody, RtScope},
    header::{CMessageSegmentHeader, GetRequestFlags, SegmentHeaderCommonFlags},
    link::{LinkSegment, LinkSegmentBody},
    CSegmentType, NlSegment,
};

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
    segments: Vec<NlSegment>,
}

impl NlMsg {
    pub const fn new(segments: Vec<NlSegment>) -> Self {
        Self { segments }
    }

    pub fn segments(&self) -> &[NlSegment] {
        &self.segments
    }

    pub fn segments_mut(&mut self) -> &mut [NlSegment] {
        &mut self.segments
    }

    pub fn read_from(reader: &mut dyn MultiRead) -> Result<Self> {
        // FIXME: Does request has only one segment? We need to do further check.
        let segments = {
            let reader = reader.current_reader_mut().ok_or_else(|| Errno::EFAULT)?;
            let segment = NlSegment::read_from(reader)?;
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
