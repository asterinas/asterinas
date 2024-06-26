// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::fs::{file_handle::FileLike, inode_handle::InodeHandle};
#[allow(non_camel_case_types)]
pub type off_t = i64;
pub const OFFSET_MAX: usize = off_t::MAX as usize;

#[derive(Debug, Copy, Clone)]
pub struct FileRange {
    start: usize,
    end: usize,
}

impl FileRange {
    /// Create the range through C flock and opened file reference
    pub fn from_c_flock_and_file(lock: &c_flock, file: Arc<dyn FileLike>) -> Result<Self> {
        let start = {
            let whence = RangeLockWhence::from_u16(lock.l_whence)?;
            match whence {
                RangeLockWhence::SEEK_SET => lock.l_start,
                RangeLockWhence::SEEK_CUR => file
                    .downcast_ref::<InodeHandle>()
                    .ok_or(Error::with_message(Errno::EBADF, "not inode"))?
                    .position()?
                    .checked_add(lock.l_start)
                    .ok_or(Error::with_message(Errno::EOVERFLOW, "start overflow"))?,

                RangeLockWhence::SEEK_END => (file.metadata().size as off_t)
                    .checked_add(lock.l_start)
                    .ok_or(Error::with_message(Errno::EOVERFLOW, "start overflow"))?,
            }
        };
        if start < 0 {
            return_errno_with_message!(Errno::EINVAL, "invalid start");
        }

        let (start, end) = match lock.l_len {
            len if len > 0 => {
                let end = start
                    .checked_add(len)
                    .ok_or(Error::with_message(Errno::EOVERFLOW, "end overflow"))?;
                (start as usize, end as usize)
            }
            0 => (start as usize, OFFSET_MAX),
            len if len < 0 => {
                let end = start;
                let new_start = start + len;
                if new_start < 0 {
                    return Err(Error::with_message(Errno::EINVAL, "invalid len"));
                }
                (new_start as usize, end as usize)
            }
            _ => unreachable!(),
        };

        Ok(Self { start, end })
    }

    pub fn new(start: usize, end: usize) -> Result<Self> {
        if start >= end {
            return_errno_with_message!(Errno::EINVAL, "invalid parameters");
        }
        Ok(Self { start, end })
    }

    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }

    pub fn set_start(&mut self, new_start: usize) -> Result<FileRangeChange> {
        if new_start >= self.end {
            return_errno_with_message!(Errno::EINVAL, "invalid new start");
        }
        let old_start = self.start;
        self.start = new_start;
        let change = match new_start {
            new_start if new_start > old_start => FileRangeChange::Shrinked,
            new_start if new_start < old_start => FileRangeChange::Expanded,
            _ => FileRangeChange::Same,
        };
        Ok(change)
    }

    pub fn set_end(&mut self, new_end: usize) -> Result<FileRangeChange> {
        if new_end <= self.start {
            return_errno_with_message!(Errno::EINVAL, "invalid new end");
        }
        let old_end = self.end;
        self.end = new_end;
        let change = match new_end {
            new_end if new_end < old_end => FileRangeChange::Shrinked,
            new_end if new_end > old_end => FileRangeChange::Expanded,
            _ => FileRangeChange::Same,
        };
        Ok(change)
    }

    pub fn overlap_with(&self, other: &Self) -> Option<OverlapWith> {
        if self.start >= other.end || self.end <= other.start {
            return None;
        }

        let overlap = if self.start <= other.start && self.end < other.end {
            OverlapWith::ToLeft
        } else if self.start > other.start && self.end < other.end {
            OverlapWith::InMiddle
        } else if self.start > other.start && self.end >= other.end {
            OverlapWith::ToRight
        } else {
            OverlapWith::Includes
        };
        Some(overlap)
    }

    pub fn merge(&mut self, other: &Self) -> Result<FileRangeChange> {
        if self.end < other.start || other.end < self.start {
            return_errno_with_message!(Errno::EINVAL, "can not merge separated ranges");
        }

        let mut change = FileRangeChange::Same;
        if other.start < self.start {
            self.start = other.start;
            change = FileRangeChange::Expanded;
        }
        if other.end > self.end {
            self.end = other.end;
            change = FileRangeChange::Expanded;
        }
        Ok(change)
    }
}

#[derive(Debug)]
pub enum FileRangeChange {
    Same,
    Expanded,
    Shrinked,
}

/// The position of a range (say A) relative another overlapping range (say B).
#[derive(Debug)]
pub enum OverlapWith {
    /// The position where range A is to the left of B (A.start <= B.start && A.end < B.end).
    ToLeft,
    /// The position where range A is to the right of B (A.start > B.start && A.end >= B.end).
    ToRight,
    /// The position where range A is in the middle of B (A.start > B.start && A.end < B.end).
    InMiddle,
    /// The position where range A includes B (A.start <= B.start && A.end >= B.end).
    Includes,
}

#[allow(non_camel_case_types)]
#[derive(Debug, Copy, Clone)]
#[repr(u16)]
pub enum RangeLockWhence {
    SEEK_SET = 0,
    SEEK_CUR = 1,
    SEEK_END = 2,
}

impl RangeLockWhence {
    pub fn from_u16(whence: u16) -> Result<Self> {
        Ok(match whence {
            0 => RangeLockWhence::SEEK_SET,
            1 => RangeLockWhence::SEEK_CUR,
            2 => RangeLockWhence::SEEK_END,
            _ => return_errno_with_message!(Errno::EINVAL, "Invalid whence"),
        })
    }
}
