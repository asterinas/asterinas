// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::process::Pid;

/// Builder for `RangeLockItem`.
///
/// # Example
///
/// ```no_run
/// let mut lock = RangeLockItemBuilder::new()
///     .type_(lock_type)
///     .range(from_c_flock_and_file(&lock_mut_c, file.clone())?)
///     .build()?;
/// ```
pub struct RangeLockItemBuilder {
    // Mandatory field
    type_: Option<RangeLockType>,
    range: Option<FileRange>,
    // Optional fields
    owner: Option<Pid>,
    waitqueue: Option<WaitQueue>,
}

impl Default for RangeLockItemBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl RangeLockItemBuilder {
    pub fn new() -> Self {
        Self {
            owner: None,
            type_: None,
            range: None,
            waitqueue: None,
        }
    }

    pub fn owner(mut self, owner: Pid) -> Self {
        self.owner = Some(owner);
        self
    }

    pub fn type_(mut self, type_: RangeLockType) -> Self {
        self.type_ = Some(type_);
        self
    }

    pub fn range(mut self, range: FileRange) -> Self {
        self.range = Some(range);
        self
    }

    pub fn waitqueue(mut self, waitqueue: WaitQueue) -> Self {
        self.waitqueue = Some(waitqueue);
        self
    }

    pub fn build(self) -> Result<RangeLockItem> {
        let owner = self.owner.unwrap_or_else(|| current!().pid());
        let type_ = if let Some(type_) = self.type_ {
            type_
        } else {
            return_errno_with_message!(Errno::EINVAL, "type_ is mandatory");
        };
        let range = if let Some(range) = self.range {
            range
        } else {
            return_errno_with_message!(Errno::EINVAL, "range is mandatory");
        };
        let waitqueue = match self.waitqueue {
            Some(waitqueue) => Arc::new(waitqueue),
            None => Arc::new(WaitQueue::new()),
        };
        Ok(RangeLockItem {
            lock: RangeLock {
                owner,
                type_,
                range,
            },
            waitqueue,
        })
    }
}
