// SPDX-License-Identifier: MPL-2.0
use alloc::fmt;
use core::ptr;

use ostd::sync::{WaitQueue, Waiter, Waker};

use crate::{
    fs::{file_handle::FileLike, inode_handle::InodeHandle},
    prelude::*,
};

/// Represents a file lock (FLOCK) with an owner and type.
#[derive(Debug, Clone)]
struct Flock {
    /// Owner of the lock, which is an opened file descriptor.
    owner: Weak<dyn FileLike>,
    /// Type of the lock, either shared or exclusive.
    type_: FlockType,
}

/// Represents a Flock item that can be held in a list of file locks.
/// Each FlockItem contains a lock and a wait queue for threads that are blocked by the lock.
pub struct FlockItem {
    lock: Flock,
    /// A wait queue for any threads that are blocked by this lock.
    waitqueue: Arc<WaitQueue>,
}

impl FlockItem {
    /// Creates a new FlockItem with the specified owner and lock type.
    pub fn new(owner: &Arc<dyn FileLike>, type_: FlockType) -> Self {
        Self {
            lock: Flock {
                owner: Arc::downgrade(owner),
                type_,
            },
            waitqueue: Arc::new(WaitQueue::new()),
        }
    }

    /// Returns the owner of the lock if it exists.
    pub fn owner(&self) -> Option<Arc<dyn FileLike>> {
        Weak::upgrade(&self.lock.owner)
    }

    /// Checks if this lock has the same owner as another lock.
    pub fn same_owner_with(&self, other: &Self) -> bool {
        self.lock.owner.ptr_eq(&other.lock.owner)
    }

    /// Returns true if this lock conflicts with another lock.
    /// Two locks conflict if they have different owners and at least one of them is an exclusive lock.
    pub fn conflict_with(&self, other: &Self) -> bool {
        if self.same_owner_with(other) {
            return false;
        }
        if self.lock.type_ == FlockType::ExclusiveLock
            || other.lock.type_ == FlockType::ExclusiveLock
        {
            return true;
        }
        false
    }

    /// Wakes all threads that are waiting for this lock.
    pub fn wake_all(&self) {
        self.waitqueue.wake_all();
    }
}

impl Clone for FlockItem {
    fn clone(&self) -> Self {
        Self {
            lock: self.lock.clone(),
            waitqueue: self.waitqueue.clone(),
        }
    }
}

/// When a FlockItem is dropped, it wakes all threads that are waiting for the lock.
impl Drop for FlockItem {
    fn drop(&mut self) {
        self.waitqueue.wake_all();
    }
}

impl Debug for FlockItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Flock")
            .field("owner", &self.lock.owner.as_ptr())
            .field("type_", &self.lock.type_)
            .finish()
    }
}

/// Represents a list of non-POSIX file advisory locks (FLOCK).
/// The list is used to manage file locks and resolve conflicts between them.
pub struct FlockList {
    inner: Mutex<Vec<FlockItem>>,
}

impl FlockList {
    /// Creates a new FlockList.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
        }
    }

    /// Attempts to set a lock on the file.
    ///
    /// If no conflicting locks exist, the lock is set and the function returns `Ok(())`.
    /// If a conflicting lock exists:
    /// - If waker is not `None`, it is added to the conflicting lock's waitqueue, and the function returns `EAGAIN`.
    /// - If waker is `None`, the function returns `EAGAIN`.
    fn try_set_lock(&self, req_lock: &FlockItem, waker: Option<&Arc<Waker>>) -> Result<()> {
        let mut list = self.inner.lock();
        if let Some(conflict_lock) = list.iter().find(|l| req_lock.conflict_with(l)) {
            if let Some(waker) = waker {
                conflict_lock.waitqueue.enqueue(waker.clone());
            }
            return_errno_with_message!(Errno::EAGAIN, "the file is locked");
        } else {
            match list.iter().position(|l| req_lock.same_owner_with(l)) {
                Some(idx) => {
                    list[idx] = req_lock.clone();
                }
                None => {
                    list.push(req_lock.clone());
                }
            }
            Ok(())
        }
    }

    /// Sets a lock on the file.
    ///
    /// If no conflicting locks exist, the lock is set and the function returns `Ok(())`.
    /// If is_nonblocking is true and a conflicting lock exists, the function returns `EAGAIN`.
    /// Otherwise, the function waits until the lock can be acquired or until it is interrupted by a signal.
    pub fn set_lock(&self, req_lock: FlockItem, is_nonblocking: bool) -> Result<()> {
        debug!(
            "set_lock with Flock: {:?}, is_nonblocking: {}",
            req_lock, is_nonblocking
        );
        if is_nonblocking {
            self.try_set_lock(&req_lock, None)
        } else {
            let (waiter, waker) = Waiter::new_pair();
            waiter.pause_until(|| {
                let result = self.try_set_lock(&req_lock, Some(&waker));
                if result.is_err_and(|err| err.error() == Errno::EAGAIN) {
                    None
                } else {
                    Some(result)
                }
            })?
        }
    }

    /// Unlocks the specified owner, waking any waiting threads.
    /// If the owner is no longer valid, the lock is removed from the list.
    /// If the owner is valid, the lock is removed from the list and all threads waiting for the lock are woken.
    /// The function does nothing if the owner is not found in the list.
    /// The function is called when the file is closed or the lock is released.
    pub fn unlock<R>(&self, req_owner: &InodeHandle<R>) {
        debug!(
            "unlock with owner: {:?}",
            req_owner as *const InodeHandle<R>
        );
        let mut list = self.inner.lock();
        list.retain(|lock| {
            if let Some(owner) = lock.owner() {
                if ptr::eq(
                    Arc::as_ptr(&owner) as *const InodeHandle<R>,
                    req_owner as *const InodeHandle<R>,
                ) {
                    lock.wake_all(); // Wake all threads waiting for this lock.
                    false // Remove lock from the list.
                } else {
                    true // Keep lock in the list.
                }
            } else {
                false // Remove lock if the owner is no longer valid.
            }
        });
    }
}

impl Default for FlockList {
    fn default() -> Self {
        Self::new()
    }
}

/// Represents the type of a Flock - either shared or exclusive.
#[derive(Debug, Copy, Clone, PartialEq)]
#[repr(u16)]
pub enum FlockType {
    /// Represents a shared lock.
    SharedLock = 0,
    /// Represents an exclusive lock.
    ExclusiveLock = 1,
}
