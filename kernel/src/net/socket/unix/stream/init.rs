// SPDX-License-Identifier: MPL-2.0

use super::{connected::Connected, listener::Listener};
use crate::{
    events::{IoEvents, Observer},
    net::socket::unix::addr::{UnixSocketAddr, UnixSocketAddrBound},
    prelude::*,
    process::signal::{Pollee, Poller},
};

pub(super) struct Init {
    addr: Option<UnixSocketAddrBound>,
    reader_pollee: Pollee,
    writer_pollee: Pollee,
}

impl Init {
    pub(super) fn new() -> Self {
        Self {
            addr: None,
            reader_pollee: Pollee::new(IoEvents::empty()),
            writer_pollee: Pollee::new(IoEvents::empty()),
        }
    }

    pub(super) fn bind(&mut self, addr_to_bind: UnixSocketAddr) -> Result<()> {
        if self.addr.is_some() {
            return_errno_with_message!(Errno::EINVAL, "the socket is already bound");
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
        } = self;

        Connected::new_pair(
            addr,
            Some(peer_addr),
            Some(reader_pollee),
            Some(writer_pollee),
        )
    }

    pub(super) fn listen(self, backlog: usize) -> core::result::Result<Listener, (Error, Self)> {
        let Some(addr) = self.addr else {
            return Err((
                Error::with_message(Errno::EINVAL, "the socket is not bound"),
                self,
            ));
        };

        // There is no `writer_pollee` in `Listener`.
        Ok(Listener::new(addr, self.reader_pollee, backlog))
    }

    pub(super) fn addr(&self) -> Option<&UnixSocketAddrBound> {
        self.addr.as_ref()
    }

    pub(super) fn poll(&self, mask: IoEvents, mut poller: Option<&mut Poller>) -> IoEvents {
        // To avoid loss of events, this must be compatible with
        // `Connected::poll`/`Listener::poll`.
        self.reader_pollee.poll(mask, poller.as_deref_mut());
        self.writer_pollee.poll(mask, poller);

        (IoEvents::OUT | IoEvents::HUP) & (mask | IoEvents::ALWAYS_POLL)
    }

    pub(super) fn register_observer(
        &self,
        observer: Weak<dyn Observer<IoEvents>>,
        mask: IoEvents,
    ) -> Result<()> {
        // To avoid loss of events, this must be compatible with
        // `Connected::poll`/`Listener::poll`.
        self.reader_pollee.register_observer(observer.clone(), mask);
        self.writer_pollee.register_observer(observer, mask);
        Ok(())
    }

    pub(super) fn unregister_observer(
        &self,
        observer: &Weak<dyn Observer<IoEvents>>,
    ) -> Option<Weak<dyn Observer<IoEvents>>> {
        let reader_observer = self.reader_pollee.unregister_observer(observer);
        let writer_observer = self.writer_pollee.unregister_observer(observer);
        reader_observer.or(writer_observer)
    }
}
