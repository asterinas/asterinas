// SPDX-License-Identifier: MPL-2.0

use alloc::fmt;
use core::ptr;

use ostd::sync::WaitQueue;

use crate::{
    fs::{file_handle::FileLike, inode_handle::InodeHandle},
    prelude::*,
};

/// Kernel representation of FLOCK
pub struct Flock {
    /// Owner of FLOCK, an opened file descriptor holding the lock
    owner: Weak<dyn FileLike>,
    /// Type of lock, SH_LOCK or EX_LOCK
    type_: FlockType,
    /// Optional waiters that are blocking by the lock
    waitqueue: Option<WaitQueue>,
}

impl Flock {
    pub fn new(owner: &Arc<dyn FileLike>, type_: FlockType) -> Self {
        Self {
            owner: Arc::downgrade(owner),
            type_,
            waitqueue: None,
        }
    }

    pub fn owner(&self) -> Option<Arc<dyn FileLike>> {
        Weak::upgrade(&self.owner)
    }

    pub fn same_owner_with(&self, other: &Self) -> bool {
        self.owner.ptr_eq(&other.owner)
    }

    pub fn conflict_with(&self, other: &Self) -> bool {
        if self.same_owner_with(other) {
            return false;
        }
        if self.type_ == FlockType::EX_LOCK || other.type_ == FlockType::EX_LOCK {
            return true;
        }
        false
    }

    pub fn wait(&mut self) {
        if self.waitqueue.is_none() {
            self.waitqueue = Some(WaitQueue::new());
        }
        let cond = || None::<()>;
        self.waitqueue.as_ref().unwrap().wait_until(cond);
    }

    pub fn wake_all(&mut self) -> usize {
        if let Some(waitqueue) = &self.waitqueue {
            waitqueue.wake_all();
        }
        0
    }
}

impl Drop for Flock {
    fn drop(&mut self) {
        self.wake_all();
    }
}

impl Debug for Flock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Flock")
            .field("owner", &self.owner.as_ptr())
            .field("type_", &self.type_)
            .finish()
    }
}

/// List of Non-POSIX file advisory lock (FLOCK)
pub struct FlockList {
    inner: RwLock<VecDeque<Flock>>,
}

impl FlockList {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(VecDeque::new()),
        }
    }

    pub fn set_lock(&self, mut req_lock: Flock, is_nonblocking: bool) -> Result<()> {
        debug!(
            "set_lock with Flock: {:?}, is_nonblocking: {}",
            req_lock, is_nonblocking
        );

        loop {
            let mut list = self.inner.write();
            if let Some(conflict_lock) = list.iter_mut().find(|l| req_lock.conflict_with(l)) {
                if is_nonblocking {
                    return_errno_with_message!(Errno::EAGAIN, "The file is locked");
                }
                // Ensure that we drop any locks before wait
                conflict_lock.wake_all();
                conflict_lock.wait();
                // Wake up, let's try to set lock again
                continue;
            }
            match list.iter().position(|l| req_lock.same_owner_with(l)) {
                Some(idx) => {
                    core::mem::swap(&mut req_lock, &mut list[idx]);
                }
                None => {
                    list.push_front(req_lock);
                }
            }
            break;
        }
        Ok(())
    }

    pub fn unlock(&self, req_owner: &InodeHandle) {
        debug!("unlock with owner: {:?}", req_owner as *const InodeHandle);

        let mut list = self.inner.write();
        list.retain(|lock| {
            if let Some(owner) = lock.owner() {
                !ptr::eq(
                    Arc::as_ptr(&owner) as *const InodeHandle,
                    req_owner as *const InodeHandle,
                )
            } else {
                false
            }
        });
    }
}

impl Default for FlockList {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(non_camel_case_types)]
#[derive(Debug, Copy, Clone, PartialEq)]
#[repr(u16)]
pub enum FlockType {
    /// Shared lock
    SH_LOCK = 0,
    /// Exclusive lock
    EX_LOCK = 1,
}

impl From<FlockOps> for FlockType {
    fn from(ops: FlockOps) -> Self {
        if ops.contains(FlockOps::LOCK_EX) {
            Self::EX_LOCK
        } else if ops.contains(FlockOps::LOCK_SH) {
            Self::SH_LOCK
        } else {
            panic!("invalid flockops");
        }
    }
}

bitflags! {
    pub struct FlockOps: i32 {
        /// Shared lock
        const LOCK_SH = 1;
        /// Exclusive lock
        const LOCK_EX = 2;
        // Or'd with one of the above to prevent blocking
        const LOCK_NB = 4;
        // Remove lock
        const LOCK_UN = 8;
    }
}

impl FlockOps {
    pub fn from_i32(bits: i32) -> Result<Self> {
        if let Some(ops) = Self::from_bits(bits) {
            if ops.contains(Self::LOCK_SH) {
                if ops.contains(Self::LOCK_EX) || ops.contains(Self::LOCK_UN) {
                    return_errno_with_message!(Errno::EINVAL, "invalid operation");
                }
            } else if ops.contains(Self::LOCK_EX) {
                if ops.contains(Self::LOCK_UN) {
                    return_errno_with_message!(Errno::EINVAL, "invalid operation");
                }
            } else if !ops.contains(Self::LOCK_UN) {
                return_errno_with_message!(Errno::EINVAL, "invalid operation");
            }
            Ok(ops)
        } else {
            return_errno_with_message!(Errno::EINVAL, "invalid operation");
        }
    }
}
