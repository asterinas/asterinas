// SPDX-License-Identifier: MPL-2.0

use crate::{
    events::IoEvents,
    fs::utils::{Channel, Consumer, Producer},
    net::socket::{unix::addr::UnixSocketAddrBound, SockShutdownCmd},
    prelude::*,
    process::signal::Poller,
};

pub(super) struct Endpoint {
    addr: Option<UnixSocketAddrBound>,
    peer_addr: Option<UnixSocketAddrBound>,
    reader: Consumer<u8>,
    writer: Producer<u8>,
}

impl Endpoint {
    pub(super) fn new_pair(
        addr: Option<UnixSocketAddrBound>,
        peer_addr: Option<UnixSocketAddrBound>,
    ) -> (Endpoint, Endpoint) {
        let (writer_this, reader_peer) = Channel::new(DAFAULT_BUF_SIZE).split();
        let (writer_peer, reader_this) = Channel::new(DAFAULT_BUF_SIZE).split();

        let this = Endpoint {
            addr: addr.clone(),
            peer_addr: peer_addr.clone(),
            reader: reader_this,
            writer: writer_this,
        };
        let peer = Endpoint {
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
        self.reader.try_read(buf)
    }

    pub(super) fn try_write(&self, buf: &[u8]) -> Result<usize> {
        self.writer.try_write(buf)
    }

    pub(super) fn shutdown(&self, cmd: SockShutdownCmd) -> Result<()> {
        // FIXME: If the socket has already been shut down, should we return an error code?

        if cmd.shut_read() {
            self.reader.shutdown();
        }

        if cmd.shut_write() {
            self.writer.shutdown();
        }

        Ok(())
    }

    pub(super) fn poll(&self, mask: IoEvents, mut poller: Option<&mut Poller>) -> IoEvents {
        let mut events = IoEvents::empty();

        // FIXME: should reader and writer use the same mask?
        let reader_events = self.reader.poll(mask, poller.as_deref_mut());
        let writer_events = self.writer.poll(mask, poller);

        // FIXME: Check this logic later.
        if reader_events.contains(IoEvents::HUP) || self.reader.is_shutdown() {
            events |= IoEvents::RDHUP | IoEvents::IN;
            if writer_events.contains(IoEvents::ERR) || self.writer.is_shutdown() {
                events |= IoEvents::HUP | IoEvents::OUT;
            }
        }

        events |= (reader_events & IoEvents::IN) | (writer_events & IoEvents::OUT);

        events
    }
}

const DAFAULT_BUF_SIZE: usize = 4096;
