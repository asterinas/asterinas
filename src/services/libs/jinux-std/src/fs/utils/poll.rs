#![allow(non_camel_case_types)]

use super::IoEvents;
use crate::fs::file_table::FileDescripter;
use crate::prelude::*;
pub type c_nfds = u64;

// https://github.com/torvalds/linux/blob/master/include/uapi/asm-generic/poll.h
#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct c_pollfd {
    fd: FileDescripter,
    events: i16,
    revents: i16,
}

#[derive(Debug, Clone, Copy)]
pub struct PollFd {
    pub fd: FileDescripter,
    pub events: IoEvents,
    pub revents: IoEvents,
}

impl From<c_pollfd> for PollFd {
    fn from(raw: c_pollfd) -> Self {
        let events = IoEvents::from_bits_truncate(raw.events as _);
        let revents = IoEvents::from_bits_truncate(raw.revents as _);
        Self {
            fd: raw.fd,
            events,
            revents,
        }
    }
}

impl From<PollFd> for c_pollfd {
    fn from(raw: PollFd) -> Self {
        let events = raw.events.bits() as i16;
        let revents = raw.revents.bits() as i16;
        Self {
            fd: raw.fd,
            events,
            revents,
        }
    }
}
