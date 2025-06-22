// SPDX-License-Identifier: MPL-2.0

//! Netlink message types for all netlink protocols.
//!
//! This module defines how to interpret messages sent from user space and how to write
//! kernel messages back to user space.

mod attr;
mod segment;

pub(super) use attr::{noattr::NoAttr, Attribute, CAttrHeader};
pub(super) use segment::{
    ack::{DoneSegment, ErrorSegment},
    common::SegmentCommon,
    header::{CMsgSegHdr, GetRequestFlags, SegHdrCommonFlags},
    CSegmentType, SegmentBody,
};

use crate::{
    prelude::*,
    util::{MultiRead, MultiWrite},
};

/// A netlink message.
///
/// A netlink message can be transmitted to and from user space using a single send/receive syscall.
/// It consists of one or more [`ProtocolSegment`]s.
#[derive(Debug)]
pub struct Message<T: ProtocolSegment> {
    segments: Vec<T>,
}

impl<T: ProtocolSegment> Message<T> {
    pub(super) const fn new(segments: Vec<T>) -> Self {
        Self { segments }
    }

    pub(super) fn segments(&self) -> &[T] {
        &self.segments
    }

    pub(super) fn segments_mut(&mut self) -> &mut [T] {
        &mut self.segments
    }

    pub(super) fn read_from(reader: &mut dyn MultiRead) -> Result<Self> {
        // FIXME: Does a request contain only one segment? We need to investigate further.
        let segments = {
            let segment = T::read_from(reader)?;
            vec![segment]
        };

        Ok(Self { segments })
    }

    pub(super) fn write_to(&self, writer: &mut dyn MultiWrite) -> Result<()> {
        for segment in self.segments.iter() {
            segment.write_to(writer)?;
        }

        Ok(())
    }

    pub(super) fn total_len(&self) -> usize {
        self.segments
            .iter()
            .map(|segment| segment.header().len as usize)
            .sum()
    }
}

pub trait ProtocolSegment: Sized {
    fn header(&self) -> &CMsgSegHdr;
    fn header_mut(&mut self) -> &mut CMsgSegHdr;
    fn read_from(reader: &mut dyn MultiRead) -> Result<Self>;
    fn write_to(&self, writer: &mut dyn MultiWrite) -> Result<()>;
}

pub(super) const NLMSG_ALIGN: usize = 4;
