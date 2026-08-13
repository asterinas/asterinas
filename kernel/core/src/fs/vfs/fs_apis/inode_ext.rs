// SPDX-License-Identifier: MPL-2.0

use alloc::boxed::ThinBox;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::{
    fs::{
        file::flock::FlockList,
        vfs::{inode::Inode, notify::FsEventPublisher, range_lock::RangeLockList},
    },
    prelude::*,
};

/// Context for FS locks.
pub struct FsLockContext {
    range_lock_list: RangeLockList,
    flock_list: FlockList,
}

/// The inode-owned runtime claim slot used by overlayfs mounts.
pub struct OverlayInuseSlot {
    owner_token: AtomicU64,
}

impl OverlayInuseSlot {
    fn new() -> Self {
        Self {
            owner_token: AtomicU64::new(0),
        }
    }

    /// Claims this slot for a non-zero owner token.
    pub fn try_claim(&self, token: u64) -> Result<()> {
        if token == 0 {
            return_errno_with_message!(Errno::EINVAL, "the overlay inuse token must be non-zero");
        }
        self.owner_token
            .compare_exchange(0, token, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| Error::with_message(Errno::EBUSY, "the inode is already in use"))?;
        Ok(())
    }

    /// Releases this slot only when `token` still owns it.
    pub fn release(&self, token: u64) {
        let _ = self
            .owner_token
            .compare_exchange(token, 0, Ordering::Release, Ordering::Relaxed);
    }
}

impl FsLockContext {
    pub(self) fn new() -> Self {
        Self {
            range_lock_list: RangeLockList::new(),
            flock_list: FlockList::new(),
        }
    }

    /// Returns a reference to the range lock list.
    pub fn range_lock_list(&self) -> &RangeLockList {
        &self.range_lock_list
    }

    /// Returns a reference to the flock list.
    pub fn flock_list(&self) -> &FlockList {
        &self.flock_list
    }
}

/// A trait that instantiates kernel types for the inode [`Extension`].
///
/// [`Extension`]: super::inode::Extension
pub trait InodeExt {
    /// Gets or initializes the FS event publisher.
    ///
    /// If the publisher does not exist for this inode, it will be created.
    fn fs_event_publisher_or_init(&self) -> &FsEventPublisher;

    /// Returns a reference to the FS event publisher.
    ///
    /// If the publisher does not exist for this inode, a [`None`] will be returned.
    fn fs_event_publisher(&self) -> Option<&FsEventPublisher>;

    /// Gets or initializes the FS lock context.
    ///
    /// If the context does not exist for this inode, it will be created.
    fn fs_lock_context_or_init(&self) -> &FsLockContext;

    /// Returns a reference to the FS lock context.
    ///
    /// If the context does not exist for this inode, a [`None`] will be returned.
    fn fs_lock_context(&self) -> Option<&FsLockContext>;

    /// Gets or initializes the overlayfs in-use slot.
    fn overlay_inuse_slot(&self) -> &OverlayInuseSlot;
}

impl InodeExt for dyn Inode {
    fn fs_event_publisher_or_init(&self) -> &FsEventPublisher {
        self.extension()
            .group1()
            .call_once(|| ThinBox::new_unsize(FsEventPublisher::new()))
            .downcast_ref()
            .unwrap()
    }

    fn fs_event_publisher(&self) -> Option<&FsEventPublisher> {
        Some(self.extension().group1().get()?.downcast_ref().unwrap())
    }

    fn fs_lock_context_or_init(&self) -> &FsLockContext {
        self.extension()
            .group2()
            .call_once(|| ThinBox::new_unsize(FsLockContext::new()))
            .downcast_ref()
            .unwrap()
    }

    fn fs_lock_context(&self) -> Option<&FsLockContext> {
        Some(self.extension().group2().get()?.downcast_ref().unwrap())
    }

    fn overlay_inuse_slot(&self) -> &OverlayInuseSlot {
        match self
            .extension()
            .group3()
            .call_once(|| ThinBox::new_unsize(OverlayInuseSlot::new()))
            .downcast_ref()
        {
            Some(slot) => slot,
            None => {
                unreachable!("the dedicated overlay inuse extension group has the wrong payload")
            }
        }
    }
}
