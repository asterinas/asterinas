// SPDX-License-Identifier: MPL-2.0

//! Re-exports used throughout the ext2 module.

pub(super) use core::{
    ops::{Deref, DerefMut, Range},
    time::Duration,
};

pub(super) use align_ext::AlignExt;
pub(super) use aster_block::{
    BLOCK_SIZE, BlockDevice, SECTOR_SIZE,
    bio::{BioDirection, BioSegment, BioStatus},
    id::Bid,
};
pub(super) use io_util::batch::IoBatch;
pub(super) use ostd::{
    mm::{Frame, FrameAllocOptions, Segment, USegment, VmIo, VmIoFill},
    sync::{RwMutex, RwMutexReadGuard, RwMutexWriteGuard},
};

pub(super) use super::{
    inode::{Ext2Bid, Ext2Ino, Iblock},
    utils::{Dirty, IsPowerOf},
};
pub(super) use crate::{
    fs::{
        file::InodeType,
        utils::{DirentVisitor, Str16, Str64},
    },
    prelude::*,
    time::UnixTime,
    vm::page_cache::{BlockAsPageCacheBackend, PageCache, PageCacheBackend},
};
