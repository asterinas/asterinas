// SPDX-License-Identifier: MPL-2.0

use super::file_table::FileDesc;
use crate::{events::IoEvents, prelude::*};

mod entry;
mod file;

pub use file::EpollFile;

/// An epoll control command.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum EpollCtl {
    Add(FileDesc, EpollEvent, EpollFlags),
    Del(FileDesc),
    Mod(FileDesc, EpollEvent, EpollFlags),
}

bitflags! {
    /// Linux's epoll flags.
    pub struct EpollFlags: u32 {
        const EXCLUSIVE      = (1 << 28);
        const WAKE_UP        = (1 << 29);
        const ONE_SHOT       = (1 << 30);
        const EDGE_TRIGGER   = (1 << 31);
    }
}

/// An epoll event.
///
/// This could be used as either an input of epoll ctl or an output of epoll wait.
/// The memory layout is compatible with that of C's struct epoll_event.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct EpollEvent {
    /// I/O events.
    ///
    /// When `EpollEvent` is used as inputs, this is treated as a mask of events.
    /// When `EpollEvent` is used as outputs, this is the active events.
    pub events: IoEvents,
    /// A 64-bit, user-given data.
    pub user_data: u64,
}

impl EpollEvent {
    /// Create a new epoll event.
    pub fn new(events: IoEvents, user_data: u64) -> Self {
        Self { events, user_data }
    }
}
