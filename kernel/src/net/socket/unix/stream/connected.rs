// SPDX-License-Identifier: MPL-2.0

use crate::{
    events::{IoEvents, Observer},
    fs::utils::{Channel, Consumer, Producer},
    net::socket::{unix::addr::UnixSocketAddrBound, SockShutdownCmd},
    prelude::*,
    process::signal::{Pollee, Poller},
};

pub(super) struct Connected {
    addr: Option<UnixSocketAddrBound>,
    peer_addr: Option<UnixSocketAddrBound>,
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

        let this = Connected {
            addr: addr.clone(),
            peer_addr: peer_addr.clone(),
            reader: reader_this,
            writer: writer_this,
        };
        let peer = Connected {
            addr: peer_addr,
            peer_addr: addr,
            reader: reader_peer,
            writer: writer_peer,
        };

        (this, peer)
    }

    pub(super) fn addr(&self) -> Option<&UnixSocketAddrBound> {
        self.addr.as_ref()
    }

    pub(super) fn peer_addr(&self) -> Option<&UnixSocketAddrBound> {
        self.peer_addr.as_ref()
    }

    pub(super) fn try_read(&self, buf: &mut [u8]) -> Result<usize> {
        let mut writer = VmWriter::from(buf).to_fallible();
        self.reader.try_read(&mut writer)
    }

    pub(super) fn try_write(&self, buf: &[u8]) -> Result<usize> {
        let mut reader = VmReader::from(buf).to_fallible();
        self.writer.try_write(&mut reader)
    }

    pub(super) fn shutdown(&self, cmd: SockShutdownCmd) {
        if cmd.shut_read() {
            self.reader.shutdown();
        }

        if cmd.shut_write() {
            self.writer.shutdown();
        }
    }

    pub(super) fn poll(&self, mask: IoEvents, mut poller: Option<&mut Poller>) -> IoEvents {
        // Note that `mask | IoEvents::ALWAYS_POLL` contains all the events we care about.
        let reader_events = self.reader.poll(mask, poller.as_deref_mut());
        let writer_events = self.writer.poll(mask, poller);

        combine_io_events(mask, reader_events, writer_events)
    }

    pub(super) fn register_observer(
        &self,
        observer: Weak<dyn Observer<IoEvents>>,
        mask: IoEvents,
    ) -> Result<()> {
        self.reader.register_observer(observer.clone(), mask)?;
        self.writer.register_observer(observer, mask)?;
        Ok(())
    }

    pub(super) fn unregister_observer(
        &self,
        observer: &Weak<dyn Observer<IoEvents>>,
    ) -> Option<Weak<dyn Observer<IoEvents>>> {
        let reader_observer = self.reader.unregister_observer(observer);
        let writer_observer = self.writer.unregister_observer(observer);
        reader_observer.or(writer_observer)
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

const DEFAULT_BUF_SIZE: usize = 65536;
