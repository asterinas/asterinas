// SPDX-License-Identifier: MPL-2.0

use crate::{
    events::IoEvents,
    fs::utils::{Channel, Consumer, Producer, StatusFlags},
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
        is_nonblocking: bool,
    ) -> Result<(Endpoint, Endpoint)> {
        let flags = if is_nonblocking {
            StatusFlags::O_NONBLOCK
        } else {
            StatusFlags::empty()
        };

        let (writer_this, reader_peer) =
            Channel::with_capacity_and_flags(DAFAULT_BUF_SIZE, flags)?.split();
        let (writer_peer, reader_this) =
            Channel::with_capacity_and_flags(DAFAULT_BUF_SIZE, flags)?.split();

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

        Ok((this, peer))
    }

    pub(super) fn addr(&self) -> Option<&UnixSocketAddrBound> {
        self.addr.as_ref()
    }

    pub(super) fn peer_addr(&self) -> Option<&UnixSocketAddrBound> {
        self.peer_addr.as_ref()
    }

    pub(super) fn is_nonblocking(&self) -> bool {
        let reader_status = self.reader.is_nonblocking();
        let writer_status = self.writer.is_nonblocking();

        debug_assert!(reader_status == writer_status);

        reader_status
    }

    pub(super) fn set_nonblocking(&self, is_nonblocking: bool) -> Result<()> {
        let mut reader_flags = self.reader.status_flags();
        reader_flags.set(StatusFlags::O_NONBLOCK, is_nonblocking);
        self.reader.set_status_flags(reader_flags)?;

        let mut writer_flags = self.writer.status_flags();
        writer_flags.set(StatusFlags::O_NONBLOCK, is_nonblocking);
        self.writer.set_status_flags(writer_flags)?;

        Ok(())
    }

    pub(super) fn read(&self, buf: &mut [u8]) -> Result<usize> {
        self.reader.read(buf)
    }

    pub(super) fn write(&self, buf: &[u8]) -> Result<usize> {
        self.writer.write(buf)
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
