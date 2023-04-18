use super::file_table::FileDescripter;
use super::utils::IoEvents;
use crate::prelude::*;

mod epoll_file;

pub use self::epoll_file::EpollFile;

/// An epoll control command.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum EpollCtl {
    Add(FileDescripter, EpollEvent, EpollFlags),
    Del(FileDescripter),
    Mod(FileDescripter, EpollEvent, EpollFlags),
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

impl From<&c_epoll_event> for EpollEvent {
    fn from(c_event: &c_epoll_event) -> Self {
        Self {
            events: IoEvents::from_bits_truncate(c_event.events as u32),
            user_data: c_event.data,
        }
    }
}

impl From<&EpollEvent> for c_epoll_event {
    fn from(ep_event: &EpollEvent) -> Self {
        Self {
            events: ep_event.events.bits() as u32,
            data: ep_event.user_data,
        }
    }
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct c_epoll_event {
    pub events: u32,
    pub data: u64,
}
