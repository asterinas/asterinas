// SPDX-License-Identifier: MPL-2.0

use super::*;

/// The maximum offset in a file.
pub const OFFSET_MAX: usize = i64::MAX as usize;

/// A range in a file.
///
/// The range is [start, end).
/// The range is valid if start < end.
/// The range is empty if start == end.
/// The range is [0, OFFSET_MAX] if it is not set.
/// The range is [start, OFFSET_MAX] if only start is set.
/// The range is [0, end] if only end is set.
/// The range is [start, end] if both start and end are set.
#[derive(Debug, Copy, Clone)]
pub struct FileRange {
    start: usize,
    end: usize,
}

impl FileRange {
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
            new_start if new_start > old_start => FileRangeChange::Shrunk,
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
            new_end if new_end < old_end => FileRangeChange::Shrunk,
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
    Shrunk,
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
