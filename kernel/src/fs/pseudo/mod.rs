// SPDX-License-Identifier: MPL-2.0

use ostd::mm::PAGE_SIZE;

use crate::fs::utils::SuperBlock;

mod anon_inodefs;
mod epoll;
mod event;
mod signal;

pub use anon_inodefs::{alloc_anon_dentry, AnonInodeFs};
pub use epoll::{EpollCtl, EpollEvent, EpollFile, EpollFlags};
pub use event::{EventFile, Flags};
pub use signal::SignalFile;

const MAX_NAME_LEN: usize = 255;

pub(super) fn alloc_pseudo_superblock(magic: u64) -> SuperBlock {
    SuperBlock::new(magic, PAGE_SIZE, MAX_NAME_LEN)
}
