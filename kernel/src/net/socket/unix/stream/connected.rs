// SPDX-License-Identifier: MPL-2.0

use core::ops::Deref;

use ostd::sync::PreemptDisabled;

use crate::{
    events::IoEvents,
    fs::utils::{Channel, Consumer, Producer},
    net::socket::{
        unix::{addr::UnixSocketAddrBound, UnixSocketAddr},
        SockShutdownCmd,
    },
    prelude::*,
    process::signal::{PollHandle, Pollee},
    util::{MultiRead, MultiWrite},
};

pub(super) struct Connected {
    addr: AddrView,
    reader: Consumer<u8>,
    writer: Producer<u8>,
}

impl Connected {
    pub(super) fn new_pair(
        addr: Option<UnixSocketAddrBound>,
        peer_addr: Option<UnixSocketAddrBound>,
        reader_pollee: Option<Pollee>,
        writer_pollee: Option<Pollee>,
    ) -> (Connected, Connected) {
        let (writer_peer, reader_this) =
            Channel::with_capacity_and_pollees(DEFAULT_BUF_SIZE, None, reader_pollee).split();
        let (writer_this, reader_peer) =
            Channel::with_capacity_and_pollees(DEFAULT_BUF_SIZE, writer_pollee, None).split();

        let (addr_this, addr_peer) = AddrView::new_pair(addr, peer_addr);

        let this = Connected {
            addr: addr_this,
            reader: reader_this,
            writer: writer_this,
        };
        let peer = Connected {
            addr: addr_peer,
            reader: reader_peer,
            writer: writer_peer,
        };

        (this, peer)
    }

    pub(super) fn addr(&self) -> Option<UnixSocketAddrBound> {
        self.addr.addr().deref().as_ref().cloned()
    }

    pub(super) fn peer_addr(&self) -> Option<UnixSocketAddrBound> {
        self.addr.peer_addr()
    }

    pub(super) fn bind(&self, addr_to_bind: UnixSocketAddr) -> Result<()> {
        let mut addr = self.addr.addr();

        if addr.is_some() {
            return addr_to_bind.bind_unnamed();
        }

        let bound_addr = addr_to_bind.bind()?;
        *addr = Some(bound_addr);

        Ok(())
    }

    pub(super) fn try_read(&self, writer: &mut dyn MultiWrite) -> Result<usize> {
        self.reader.try_read(writer)
    }

    pub(super) fn try_write(&self, reader: &mut dyn MultiRead) -> Result<usize> {
        self.writer.try_write(reader)
    }

    pub(super) fn shutdown(&self, cmd: SockShutdownCmd) {
        if cmd.shut_read() {
            self.reader.shutdown();
        }

        if cmd.shut_write() {
            self.writer.shutdown();
        }
    }

    pub(super) fn poll(&self, mask: IoEvents, mut poller: Option<&mut PollHandle>) -> IoEvents {
        // Note that `mask | IoEvents::ALWAYS_POLL` contains all the events we care about.
        let reader_events = self.reader.poll(mask, poller.as_deref_mut());
        let writer_events = self.writer.poll(mask, poller);

        combine_io_events(mask, reader_events, writer_events)
    }
}

pub(super) fn combine_io_events(
    mask: IoEvents,
    reader_events: IoEvents,
    writer_events: IoEvents,
) -> IoEvents {
    let mut events = IoEvents::empty();

    if reader_events.contains(IoEvents::HUP) {
        // The socket is shut down in one direction: the remote socket has shut down for
        // writing or the local socket has shut down for reading.
        events |= IoEvents::RDHUP | IoEvents::IN;

        if writer_events.contains(IoEvents::ERR) {
            // The socket is shut down in both directions. Neither reading nor writing is
            // possible.
            events |= IoEvents::HUP;
        }
    }

    events |= (reader_events & IoEvents::IN) | (writer_events & IoEvents::OUT);

    events & (mask | IoEvents::ALWAYS_POLL)
}

struct AddrView {
    addr: Arc<SpinLock<Option<UnixSocketAddrBound>>>,
    peer: Arc<SpinLock<Option<UnixSocketAddrBound>>>,
}

impl AddrView {
    fn new_pair(
        first: Option<UnixSocketAddrBound>,
        second: Option<UnixSocketAddrBound>,
    ) -> (AddrView, AddrView) {
        let first = Arc::new(SpinLock::new(first));
        let second = Arc::new(SpinLock::new(second));

        let view1 = AddrView {
            addr: first.clone(),
            peer: second.clone(),
        };
        let view2 = AddrView {
            addr: second,
            peer: first,
        };

        (view1, view2)
    }

    fn addr(&self) -> SpinLockGuard<Option<UnixSocketAddrBound>, PreemptDisabled> {
        self.addr.lock()
    }

    fn peer_addr(&self) -> Option<UnixSocketAddrBound> {
        self.peer.lock().as_ref().cloned()
    }
}

const DEFAULT_BUF_SIZE: usize = 65536;
