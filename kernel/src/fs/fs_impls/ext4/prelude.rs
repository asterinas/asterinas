// SPDX-License-Identifier: MPL-2.0

//! Re-exports used throughout the ext4 module.

pub(super) use core::{
    ops::{Deref, DerefMut, Range},
    time::Duration,
};

pub(super) use align_ext::AlignExt;
pub(super) use aster_block::{
    BLOCK_SIZE, BlockDevice, SECTOR_SIZE,
    bio::{BioCompleteFn, BioDirection, BioSegment, BioStatus},
    id::Bid,
};
pub(super) use io_util::batch::IoBatch;
pub(super) use ostd::{
    const_assert,
    mm::{Frame, FrameAllocOptions, PAGE_SIZE, Segment, USegment, VmIo, VmIoFill, VmWriter},
    sync::{Mutex, MutexGuard, RwMutex, RwMutexReadGuard},
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
