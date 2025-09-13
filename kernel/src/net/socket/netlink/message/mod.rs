// SPDX-License-Identifier: MPL-2.0

//! Netlink message types for all netlink protocols.
//!
//! This module defines how to interpret messages sent from user space and how to write
//! kernel messages back to user space.

mod attr;
mod result;
mod segment;

pub(super) use attr::{noattr::NoAttr, Attribute, CAttrHeader};
pub(super) use result::ContinueRead;
pub(super) use segment::{
    ack::{DoneSegment, ErrorSegment},
    common::SegmentCommon,
    header::{CMsgSegHdr, GetRequestFlags, SegHdrCommonFlags},
    CSegmentType, SegmentBody,
};

use super::receiver::QueueableMessage;
use crate::{
    prelude::*,
    util::{MultiRead, MultiWrite},
};

/// A netlink message.
///
/// A netlink message can be transmitted to and from user space using a single send/receive syscall.
/// It consists of one or more [`ProtocolSegment`]s.
#[derive(Debug)]
pub struct Message<T> {
    segments: Vec<T>,
}

impl<T> Message<T> {
    pub(super) const fn new(segments: Vec<T>) -> Self {
        Self { segments }
    }
}

impl<T: ProtocolSegment> Message<T> {
    // We do not provide a `read_from` method here. Netlink sockets should use `T::read_from` to
    // read the request segments one by one instead.

    pub(super) fn write_to(&self, writer: &mut dyn MultiWrite) -> Result<()> {
        for segment in self.segments.iter() {
            segment.write_to(writer)?;
        }

        Ok(())
    }
}

impl<T: ProtocolSegment> QueueableMessage for Message<T> {
    fn total_len(&self) -> usize {
        self.segments
            .iter()
            .map(|segment| segment.header().len as usize)
            .sum()
    }
}

pub trait ProtocolSegment: Sized {
    fn header(&self) -> &CMsgSegHdr;
    fn header_mut(&mut self) -> &mut CMsgSegHdr;

    /// Reads the segment body from the `reader`.
    ///
    /// If the reader encounters an unresolvable page fault, this method will fail with
    /// [`Errno::EFAULT`]. Netlink sockets should directly report this error code to the user.
    ///
    /// If the reader does not contain a valid segment header ([`CMsgSegHdr`]), this method will
    /// also fail. Netlink sockets should then stop parsing segments from the reader and silently
    /// ignore the error.
    ///
    /// If the reader contains a valid segment header but an invalid segment (e.g., one with an
    /// invalid body or attributes), this method will succeed with [`ContinueRead::Skipped`] or
    /// [`ContinueRead::SkippedErr`]. If there is an error segment in [`ContinueRead::SkippedErr`],
    /// netlink sockets should respond the user with the error segment. The entire segment is
    /// skipped so it is possible to read the next segment from the reader.
    ///
    /// This method will skip the padding bytes, so it can be called multiple times to read
    /// multiple segments.
    fn read_from(reader: &mut dyn MultiRead) -> Result<ContinueRead<Self, ErrorSegment>>;

    /// Writes the segment to the `writer`.
    ///
    /// This method will skip the padding bytes, so it can be called multiple times to write
    /// multiple segments.
    fn write_to(&self, writer: &mut dyn MultiWrite) -> Result<()>;
}

pub(super) const NLMSG_ALIGN: usize = 4;
