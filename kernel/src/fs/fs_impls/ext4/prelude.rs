// SPDX-License-Identifier: MPL-2.0

//! Re-exports used throughout the ext4 module.

pub(super) use core::{
    ops::{Deref, DerefMut},
    time::Duration,
};

pub(super) use align_ext::AlignExt;
#[cfg(ktest)]
pub(super) use aster_block::SECTOR_SIZE;
pub(super) use aster_block::{
    BLOCK_SIZE, BlockDevice,
    bio::{BioCompleteFn, BioSegment, BioStatus},
    id::Bid,
};
pub(super) use io_util::batch::IoBatch;
#[cfg(ktest)]
pub(super) use ostd::mm::{FrameAllocOptions, Segment};
pub(super) use ostd::{
    const_assert,
    mm::{PAGE_SIZE, VmIo, VmWriter},
    sync::{RwMutex, RwMutexReadGuard},
};

pub(super) use super::{
    inode::{Ext4Bid, Ext4Ino, Iblock},
    utils::Dirty,
};
pub(super) use crate::{
    fs::{
        file::InodeType,
        utils::{DirentVisitor, IdBitmap, Str16, Str64},
    },
    prelude::*,
    time::UnixTime,
    vm::page_cache::{BlockAsPageCacheBackend, PageCache, PageCacheBackend},
};
