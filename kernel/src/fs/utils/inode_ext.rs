// SPDX-License-Identifier: MPL-2.0

use alloc::boxed::ThinBox;

use super::{FlockList, Inode, RangeLockList};
use crate::fs::notify::FsEventPublisher;

/// Context for FS locks.
pub struct FsLockContext {
    range_lock_list: RangeLockList,
    flock_list: FlockList,
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
/// [`Extension`]: super::Extension
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
}
