// SPDX-License-Identifier: MPL-2.0

use alloc::{vec, vec::Vec};

use crate::{
    vm::{VmFrame, VmIo, VmReader, VmWriter, PAGE_SIZE},
    Error, Result,
};

/// A collection of base page frames (regular physical memory pages).
///
/// For the most parts, `VmFrameVec` is like `Vec<VmFrame>`. But the
/// implementation may or may not be based on `Vec`. Having a dedicated
/// type to represent a series of page frames is convenient because,
/// more often than not, one needs to operate on a batch of frames rather
/// a single frame.
#[derive(Debug, Clone)]
pub struct VmFrameVec(pub(crate) Vec<VmFrame>);

impl VmFrameVec {
    pub fn get(&self, index: usize) -> Option<&VmFrame> {
        self.0.get(index)
    }

    /// returns an empty VmFrame vec
    pub fn empty() -> Self {
        Self(Vec::new())
    }

    pub fn new_with_capacity(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    /// Pushs a new frame to the collection.
    pub fn push(&mut self, new_frame: VmFrame) {
        self.0.push(new_frame);
    }

    /// Pop a frame from the collection.
    pub fn pop(&mut self) -> Option<VmFrame> {
        self.0.pop()
    }

    /// Removes a frame at a position.
    pub fn remove(&mut self, at: usize) -> VmFrame {
        self.0.remove(at)
    }

    /// Append some frames.
    pub fn append(&mut self, more: &mut VmFrameVec) -> Result<()> {
        self.0.append(&mut more.0);
        Ok(())
    }

    /// Truncate some frames.
    ///
    /// If `new_len >= self.len()`, then this method has no effect.
    pub fn truncate(&mut self, new_len: usize) {
        if new_len >= self.0.len() {
            return;
        }
        self.0.truncate(new_len)
    }

    /// Returns an iterator
    pub fn iter(&self) -> core::slice::Iter<'_, VmFrame> {
        self.0.iter()
    }

    /// Returns the number of frames.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns whether the frame collection is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the number of bytes.
    ///
    /// This method is equivalent to `self.len() * BASE_PAGE_SIZE`.
    pub fn nbytes(&self) -> usize {
        self.0.len() * PAGE_SIZE
    }

    pub fn from_one_frame(frame: VmFrame) -> Self {
        Self(vec![frame])
    }
}

impl IntoIterator for VmFrameVec {
    type Item = VmFrame;

    type IntoIter = alloc::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl VmIo for VmFrameVec {
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        // Do bound check with potential integer overflow in mind
        let max_offset = offset.checked_add(buf.len()).ok_or(Error::Overflow)?;
        if max_offset > self.nbytes() {
            return Err(Error::InvalidArgs);
        }

        let num_unread_pages = offset / PAGE_SIZE;
        let mut start = offset % PAGE_SIZE;
        let mut buf_writer: VmWriter = buf.into();
        for frame in self.0.iter().skip(num_unread_pages) {
            let read_len = frame.reader().skip(start).read(&mut buf_writer);
            if read_len == 0 {
                break;
            }
            start = 0;
        }
        Ok(())
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        // Do bound check with potential integer overflow in mind
        let max_offset = offset.checked_add(buf.len()).ok_or(Error::Overflow)?;
        if max_offset > self.nbytes() {
            return Err(Error::InvalidArgs);
        }

        let num_unwrite_pages = offset / PAGE_SIZE;
        let mut start = offset % PAGE_SIZE;
        let mut buf_reader: VmReader = buf.into();
        for frame in self.0.iter().skip(num_unwrite_pages) {
            let write_len = frame.writer().skip(start).write(&mut buf_reader);
            if write_len == 0 {
                break;
            }
            start = 0;
        }
        Ok(())
    }
}

/// An iterator for frames.
pub struct FrameVecIter<'a> {
    frames: &'a VmFrameVec,
    current: usize,
}

impl<'a> FrameVecIter<'a> {
    pub fn new(frames: &'a VmFrameVec) -> Self {
        Self { frames, current: 0 }
    }
}

impl<'a> Iterator for FrameVecIter<'a> {
    type Item = &'a VmFrame;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current >= self.frames.0.len() {
            return None;
        }
        Some(self.frames.0.get(self.current).unwrap())
    }
}
