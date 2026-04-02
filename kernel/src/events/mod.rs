// SPDX-License-Identifier: MPL-2.0

mod epoll;
#[expect(clippy::module_inception)]
mod events;
mod io_events;
mod observer;
mod subject;

pub use self::{
    epoll::{EpollCtl, EpollEvent, EpollFile, EpollFlags},
    events::{Events, EventsFilter},
    io_events::IoEvents,
    observer::Observer,
    subject::SyncSubject,
};
