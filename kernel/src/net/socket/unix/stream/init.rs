// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use super::{
    connected::{combine_io_events, Connected},
    listener::Listener,
};
use crate::{
    events::IoEvents,
    net::socket::{
        unix::addr::{UnixSocketAddr, UnixSocketAddrBound},
        SockShutdownCmd,
    },
    prelude::*,
    process::signal::{PollHandle, Pollee},
};

pub(super) struct Init {
    addr: Option<UnixSocketAddrBound>,
    reader_pollee: Pollee,
    writer_pollee: Pollee,
    is_read_shutdown: AtomicBool,
    is_write_shutdown: AtomicBool,
}

impl Init {
    pub(super) fn new() -> Self {
        Self {
            addr: None,
            reader_pollee: Pollee::new(),
            writer_pollee: Pollee::new(),
            is_read_shutdown: AtomicBool::new(false),
            is_write_shutdown: AtomicBool::new(false),
        }
    }

    pub(super) fn bind(&mut self, addr_to_bind: UnixSocketAddr) -> Result<()> {
        if self.addr.is_some() {
            return addr_to_bind.bind_unnamed();
        }

        let bound_addr = addr_to_bind.bind()?;
        self.addr = Some(bound_addr);

        Ok(())
    }

    pub(super) fn into_connected(self, peer_addr: UnixSocketAddrBound) -> (Connected, Connected) {
        let Init {
            addr,
            reader_pollee,
            writer_pollee,
            is_read_shutdown,
            is_write_shutdown,
        } = self;

        let (this_conn, peer_conn) = Connected::new_pair(
            addr,
            Some(peer_addr),
            Some(reader_pollee),
            Some(writer_pollee),
        );

        if is_read_shutdown.into_inner() {
            this_conn.shutdown(SockShutdownCmd::SHUT_RD);
        }

        if is_write_shutdown.into_inner() {
            this_conn.shutdown(SockShutdownCmd::SHUT_WR)
        }

        (this_conn, peer_conn)
    }

    pub(super) fn listen(self, backlog: usize) -> core::result::Result<Listener, (Error, Self)> {
        let Some(addr) = self.addr else {
            return Err((
                Error::with_message(Errno::EINVAL, "the socket is not bound"),
                self,
            ));
        };

        Ok(Listener::new(
            addr,
            self.reader_pollee,
            self.writer_pollee,
            backlog,
            self.is_read_shutdown.into_inner(),
            self.is_write_shutdown.into_inner(),
        ))
    }

    pub(super) fn shutdown(&self, cmd: SockShutdownCmd) {
        match cmd {
            SockShutdownCmd::SHUT_WR | SockShutdownCmd::SHUT_RDWR => {
                self.is_write_shutdown.store(true, Ordering::Relaxed);
                self.writer_pollee.notify(IoEvents::ERR);
            }
            SockShutdownCmd::SHUT_RD => (),
        }

        match cmd {
            SockShutdownCmd::SHUT_RD | SockShutdownCmd::SHUT_RDWR => {
                self.is_read_shutdown.store(true, Ordering::Relaxed);
                self.reader_pollee.notify(IoEvents::HUP);
            }
            SockShutdownCmd::SHUT_WR => (),
        }
    }

    pub(super) fn addr(&self) -> Option<&UnixSocketAddrBound> {
        self.addr.as_ref()
    }

    pub(super) fn poll(&self, mask: IoEvents, mut poller: Option<&mut PollHandle>) -> IoEvents {
        // To avoid loss of events, this must be compatible with
        // `Connected::poll`/`Listener::poll`.
        let reader_events = self
            .reader_pollee
            .poll_with(mask, poller.as_deref_mut(), || {
                if self.is_read_shutdown.load(Ordering::Relaxed) {
                    IoEvents::HUP
                } else {
                    IoEvents::empty()
                }
            });
        let writer_events = self.writer_pollee.poll_with(mask, poller, || {
            if self.is_write_shutdown.load(Ordering::Relaxed) {
                IoEvents::OUT | IoEvents::ERR
            } else {
                IoEvents::OUT
            }
        });

        // According to the Linux implementation, we always have `IoEvents::HUP` in this state.
        // Meanwhile, it is in `IoEvents::ALWAYS_POLL`, so we always return it.
        combine_io_events(mask, reader_events, writer_events) | IoEvents::HUP
    }
}
