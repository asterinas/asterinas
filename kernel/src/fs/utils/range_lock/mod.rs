// SPDX-License-Identifier: MPL-2.0

use core::fmt;

use ostd::sync::{RwMutexWriteGuard, WaitQueue, Waiter, Waker};

use self::range::FileRangeChange;
pub use self::{
    builder::RangeLockItemBuilder,
    range::{FileRange, OverlapWith, OFFSET_MAX},
};
use crate::{prelude::*, process::Pid};

mod builder;
mod range;

/// The metadata of a POSIX advisory file range lock.
#[derive(Debug, Clone)]
struct RangeLock {
    /// Owner of the lock, representing the process holding the lock
    owner: Pid,
    /// Type of lock: can be F_RDLCK (read lock), F_WRLCK (write lock), or F_UNLCK (unlock)
    type_: RangeLockType,
    /// Range of the lock which specifies the portion of the file being locked
    range: FileRange,
}

/// Represents a POSIX advisory file range lock in the kernel.
/// Contains metadata about the lock and the processes waiting for it.
/// The lock is associated with a specific range of the file.
pub struct RangeLockItem {
    /// The lock data including its properties
    lock: RangeLock,
    /// Waiters that are being blocked by this lock
    waitqueue: Arc<WaitQueue>,
}

impl RangeLockItem {
    /// Returns the type of the lock (READ/WRITE/UNLOCK)
    pub fn type_(&self) -> RangeLockType {
        self.lock.type_
    }

    /// Sets the type of the lock to the specified type
    pub fn set_type(&mut self, type_: RangeLockType) {
        self.lock.type_ = type_;
    }

    /// Returns the owner (process ID) of the lock
    pub fn owner(&self) -> Pid {
        self.lock.owner
    }

    /// Sets the owner of the lock to the specified process ID
    pub fn set_owner(&mut self, owner: Pid) {
        self.lock.owner = owner;
    }

    /// Returns the range of the lock
    pub fn range(&self) -> FileRange {
        self.lock.range
    }

    /// Sets the range of the lock to the specified range
    pub fn set_range(&mut self, range: FileRange) {
        self.lock.range = range;
    }

    /// Checks if this lock conflicts with another lock
    /// Returns true if there is a conflict, otherwise false
    pub fn conflict_with(&self, other: &Self) -> bool {
        // If locks are owned by the same process, they do not conflict
        if self.owner() == other.owner() {
            return false;
        }
        // If the ranges do not overlap, they do not conflict
        if self.overlap_with(other).is_none() {
            return false;
        }
        // Write locks are exclusive and conflict with any other lock
        if self.type_() == RangeLockType::WriteLock || other.type_() == RangeLockType::WriteLock {
            return true;
        }
        false
    }

    /// Checks if this lock overlaps with another lock
    /// Returns an Option that contains the overlap details if they overlap
    pub fn overlap_with(&self, other: &Self) -> Option<OverlapWith> {
        self.range().overlap_with(&other.range())
    }

    /// Merges the range of this lock with another lock's range
    /// If the merge fails, it will trigger a panic
    pub fn merge_with(&mut self, other: &Self) {
        self.lock
            .range
            .merge(&other.range())
            .expect("merge range failed");
    }

    /// Returns the starting position of the lock range
    pub fn start(&self) -> usize {
        self.range().start()
    }

    /// Returns the ending position of the lock range
    pub fn end(&self) -> usize {
        self.range().end()
    }

    /// Sets a new starting position for the lock range
    /// If the range shrinks, it will wake all waiting processes
    pub fn set_start(&mut self, new_start: usize) {
        let change = self
            .lock
            .range
            .set_start(new_start)
            .expect("invalid new start");
        if let FileRangeChange::Shrunk = change {
            self.wake_all();
        }
    }

    /// Sets a new ending position for the lock range
    /// If the range shrinks, it will wake all waiting processes
    pub fn set_end(&mut self, new_end: usize) {
        let change = self.range().set_end(new_end).expect("invalid new end");
        if let FileRangeChange::Shrunk = change {
            self.wake_all();
        }
    }

    /// Puts the current process in a wait state until the lock condition is satisfied
    pub fn wait_until<F>(&self, cond: F)
    where
        F: FnMut() -> Option<()>,
    {
        self.waitqueue.wait_until(cond);
    }

    /// Wakes all the processes waiting on this lock
    /// Returns the number of processes that were woken
    pub fn wake_all(&self) -> usize {
        self.waitqueue.wake_all()
    }
}

/// Implements the drop trait for RangeLockItem
/// Ensures that all waiting processes are woken when this item goes out of scope
impl Drop for RangeLockItem {
    fn drop(&mut self) {
        self.wake_all();
    }
}

/// Implements the Debug trait for RangeLockItem
/// Customizes the output when the item is printed in debug mode
impl Debug for RangeLockItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RangeLock")
            .field("owner", &self.owner())
            .field("type_", &self.type_())
            .field("range", &self.range())
            .finish()
    }
}

/// Implements the Clone trait for RangeLockItem
/// Allows creating a copy of the item with the same properties
impl Clone for RangeLockItem {
    fn clone(&self) -> Self {
        Self {
            lock: self.lock.clone(),
            waitqueue: self.waitqueue.clone(),
        }
    }
}

/// List of File POSIX advisory range locks.
///
/// Rule of ordering:
/// Locks are sorted by owner process, then by the starting offset.
///
/// Rule of mergeing:
/// Adjacent and overlapping locks with same owner and type will be merged.
///
/// Rule of updating:
/// New locks with different type will replace or split the overlapping locks
/// if they have same owner.
pub struct RangeLockList {
    inner: RwMutex<Vec<RangeLockItem>>,
}

impl RangeLockList {
    pub fn new() -> Self {
        Self {
            inner: RwMutex::new(Vec::new()),
        }
    }

    /// Test whether `lock` may be set.
    ///
    /// If there is a conflict, return the conflicting lock.
    /// Otherwise, return a lock with type `Unlock`.
    pub fn test_lock(&self, lock: RangeLockItem) -> RangeLockItem {
        debug!("test_lock with RangeLock: {:?}", lock);
        let mut req_lock = lock.clone();
        let list = self.inner.read();
        for existing_lock in list.iter() {
            if lock.conflict_with(existing_lock) {
                req_lock.set_owner(existing_lock.owner());
                req_lock.set_type(existing_lock.type_());
                req_lock.set_range(existing_lock.range());
                return req_lock;
            }
        }
        req_lock.set_type(RangeLockType::Unlock);
        req_lock
    }

    /// Attempts to set a lock on the file.
    ///
    /// If no conflicting locks exist, the lock is set and the function returns `Ok(())`.
    /// If a conflicting lock exists:
    /// - If waker is not `None`, it is added to the conflicting lock's waitqueue, and the function returns `EAGAIN`.
    /// - If waker is `None`, the function returns `EAGAIN`.
    fn try_set_lock(&self, req_lock: &RangeLockItem, waker: Option<&Arc<Waker>>) -> Result<()> {
        let mut list = self.inner.write();
        if let Some(conflict_lock) = list.iter().find(|l| req_lock.conflict_with(l)) {
            if let Some(waker) = waker {
                conflict_lock.waitqueue.enqueue(waker.clone());
            }
            return_errno_with_message!(Errno::EAGAIN, "the file is locked");
        } else {
            Self::insert_lock_into_list(&mut list, req_lock);
            Ok(())
        }
    }

    /// Sets a lock on the file.
    ///
    /// If the lock is non-blocking and there is a conflict, return `Err(Errno::EAGAIN)`.
    /// Otherwise, block the current process until the lock can be set or it is interrupted by a signal.
    pub fn set_lock(&self, req_lock: &RangeLockItem, is_nonblocking: bool) -> Result<()> {
        debug!(
            "set_lock with RangeLock: {:?}, is_nonblocking: {}",
            req_lock, is_nonblocking
        );
        if is_nonblocking {
            self.try_set_lock(req_lock, None)
        } else {
            let (waiter, waker) = Waiter::new_pair();
            waiter.pause_until(|| {
                let result = self.try_set_lock(req_lock, Some(&waker));
                if result.is_err_and(|err| err.error() == Errno::EAGAIN) {
                    None
                } else {
                    Some(result)
                }
            })?
        }
    }

    /// Insert a lock into the list.
    fn insert_lock_into_list(
        list: &mut RwMutexWriteGuard<Vec<RangeLockItem>>,
        lock: &RangeLockItem,
    ) {
        let first_same_owner_idx = match list.iter().position(|lk| lk.owner() == lock.owner()) {
            Some(idx) => idx,
            None => {
                // Can't find existing locks with same owner.
                list.push(lock.clone());
                return;
            }
        };
        // Insert the lock at the start position with same owner, may breaking
        // the rules of RangeLockList.
        // We will handle the inserted lock with next one to adjust the list to
        // obey the rules.
        list.insert(first_same_owner_idx, lock.clone());
        let mut pre_idx = first_same_owner_idx;
        let mut next_idx = pre_idx + 1;
        loop {
            if next_idx >= list.len() {
                break;
            }

            let (left, right) = list.split_at_mut(next_idx);
            let pre_lock = &mut left[pre_idx];
            let next_lock = &mut right[0];

            if next_lock.owner() != pre_lock.owner() {
                break;
            }
            if next_lock.type_() == pre_lock.type_() {
                // Same type
                if pre_lock.end() < next_lock.start() {
                    break;
                } else if next_lock.end() < pre_lock.start() {
                    list.swap(pre_idx, next_idx);
                    pre_idx += 1;
                    next_idx += 1;
                } else {
                    // Merge adjacent or overlapping locks
                    next_lock.merge_with(pre_lock);
                    list.remove(pre_idx);
                }
            } else {
                // Different type
                if pre_lock.end() <= next_lock.start() {
                    break;
                } else if next_lock.end() <= pre_lock.start() {
                    list.swap(pre_idx, next_idx);
                    pre_idx += 1;
                    next_idx += 1;
                } else {
                    // Split overlapping locks
                    let overlap_with = pre_lock.overlap_with(next_lock).unwrap();
                    match overlap_with {
                        OverlapWith::ToLeft => {
                            next_lock.set_start(pre_lock.end());
                            break;
                        }
                        OverlapWith::InMiddle => {
                            let right_lk = {
                                let mut r_lk = next_lock.clone();
                                r_lk.set_start(pre_lock.end());
                                r_lk
                            };
                            next_lock.set_end(pre_lock.start());
                            list.swap(pre_idx, next_idx);
                            list.insert(next_idx + 1, right_lk);
                            break;
                        }
                        OverlapWith::ToRight => {
                            next_lock.set_end(pre_lock.start());
                            list.swap(pre_idx, next_idx);
                            pre_idx += 1;
                            next_idx += 1;
                        }
                        OverlapWith::Includes => {
                            // New lock can replace the old one
                            list.remove(next_idx);
                        }
                    }
                }
            }
        }
    }

    /// Unlock the lock.
    ///
    /// The lock will be removed from the list.
    /// Adjacent locks will be merged if they have the same owner and type.
    /// Overlapping locks will be split or merged if they have the same owner.
    pub fn unlock(&self, lock: &RangeLockItem) {
        debug!("unlock with RangeLock: {:?}", lock);
        let mut list = self.inner.write();
        let mut skipped = 0;
        while let Some(idx) = list
            .iter()
            .skip(skipped)
            .position(|lk| lk.owner() == lock.owner())
        {
            // (idx + skipped) is the original position in list
            let idx = idx + skipped;
            let existing_lock = &mut list[idx];

            let overlap_with = match lock.overlap_with(existing_lock) {
                Some(overlap) => overlap,
                None => {
                    skipped = idx + 1;
                    continue;
                }
            };

            match overlap_with {
                OverlapWith::ToLeft => {
                    existing_lock.set_start(lock.end());
                    break;
                }
                OverlapWith::InMiddle => {
                    // Split the lock
                    let right_lk = {
                        let mut r_lk = existing_lock.clone();
                        r_lk.set_start(lock.end());
                        r_lk
                    };
                    existing_lock.set_end(lock.start());
                    list.insert(idx + 1, right_lk);
                    break;
                }
                OverlapWith::ToRight => {
                    existing_lock.set_end(lock.start());
                    skipped = idx + 1;
                }
                OverlapWith::Includes => {
                    // The lock can be deleted from the list
                    list.remove(idx);
                    skipped = idx;
                }
            }
        }
    }
}

impl Default for RangeLockList {
    fn default() -> Self {
        Self::new()
    }
}

/// Type of file range lock, aligned with Linux kernel.
/// F_RDLCK = 0, F_WRLCK = 1, F_UNLCK = 2,
#[derive(Debug, Copy, Clone, PartialEq, TryFromInt)]
#[repr(u16)]
pub enum RangeLockType {
    ReadLock = 0,
    WriteLock = 1,
    Unlock = 2,
}
