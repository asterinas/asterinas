// SPDX-License-Identifier: MPL-2.0

mod epoll;
mod event_file;
#[expect(clippy::module_inception)]
mod events;
mod io_events;
mod observer;
mod subject;

pub(crate) use self::{
    epoll::{EpollCtl, EpollEvent, EpollFile, EpollFlags},
    event_file::{EventFile, EventFileFlags},
    events::{Events, EventsFilter},
    io_events::IoEvents,
    observer::Observer,
    subject::SyncSubject,
};
