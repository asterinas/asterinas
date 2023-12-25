use crate::events::IoEvents;
use crate::process::signal::Poller;
use crate::{
    fs::utils::{Channel, Consumer, Producer, StatusFlags},
    net::socket::{unix::addr::UnixSocketAddrBound, SockShutdownCmd},
    prelude::*,
};

pub(super) struct Endpoint(Inner);

struct Inner {
    addr: RwLock<Option<UnixSocketAddrBound>>,
    reader: Consumer<u8>,
    writer: Producer<u8>,
    peer: Weak<Endpoint>,
}

impl Endpoint {
    pub(super) fn new_pair(is_nonblocking: bool) -> Result<(Arc<Endpoint>, Arc<Endpoint>)> {
        let flags = if is_nonblocking {
            StatusFlags::O_NONBLOCK
        } else {
            StatusFlags::empty()
        };
        let (writer_a, reader_b) =
            Channel::with_capacity_and_flags(DAFAULT_BUF_SIZE, flags)?.split();
        let (writer_b, reader_a) =
            Channel::with_capacity_and_flags(DAFAULT_BUF_SIZE, flags)?.split();
        let mut endpoint_b = None;
        let endpoint_a = Arc::new_cyclic(|endpoint_a_ref| {
            let peer = Arc::new(Endpoint::new(reader_b, writer_b, endpoint_a_ref.clone()));
            let endpoint_a = Endpoint::new(reader_a, writer_a, Arc::downgrade(&peer));
            endpoint_b = Some(peer);
            endpoint_a
        });
        Ok((endpoint_a, endpoint_b.unwrap()))
    }

    fn new(reader: Consumer<u8>, writer: Producer<u8>, peer: Weak<Endpoint>) -> Self {
        Self(Inner {
            addr: RwLock::new(None),
            reader,
            writer,
            peer,
        })
    }

    pub(super) fn addr(&self) -> Option<UnixSocketAddrBound> {
        self.0.addr.read().clone()
    }

    pub(super) fn set_addr(&self, addr: UnixSocketAddrBound) {
        *self.0.addr.write() = Some(addr);
    }

    pub(super) fn peer_addr(&self) -> Option<UnixSocketAddrBound> {
        self.0.peer.upgrade().and_then(|peer| peer.addr())
    }

    pub(super) fn is_nonblocking(&self) -> bool {
        let reader_status = self.0.reader.is_nonblocking();
        let writer_status = self.0.writer.is_nonblocking();
        debug_assert!(reader_status == writer_status);
        reader_status
    }

    pub(super) fn set_nonblocking(&self, is_nonblocking: bool) -> Result<()> {
        let reader_flags = self.0.reader.status_flags();
        self.0
            .reader
            .set_status_flags(reader_flags | StatusFlags::O_NONBLOCK)?;
        let writer_flags = self.0.writer.status_flags();
        self.0
            .writer
            .set_status_flags(writer_flags | StatusFlags::O_NONBLOCK)?;
        Ok(())
    }

    pub(super) fn read(&self, buf: &mut [u8]) -> Result<usize> {
        self.0.reader.read(buf)
    }

    pub(super) fn write(&self, buf: &[u8]) -> Result<usize> {
        self.0.writer.write(buf)
    }

    pub(super) fn shutdown(&self, cmd: SockShutdownCmd) -> Result<()> {
        if !self.is_connected() {
            return_errno_with_message!(Errno::ENOTCONN, "The socket is not connected.");
        }

        if cmd.shut_read() {
            self.0.reader.shutdown();
        }

        if cmd.shut_write() {
            self.0.writer.shutdown();
        }

        Ok(())
    }

    pub(super) fn is_connected(&self) -> bool {
        self.0.peer.upgrade().is_some()
    }

    pub(super) fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        let mut events = IoEvents::empty();
        // FIXME: should reader and writer use the same mask?
        let reader_events = self.0.reader.poll(mask, poller);
        let writer_events = self.0.writer.poll(mask, poller);

        if reader_events.contains(IoEvents::HUP) || self.0.reader.is_shutdown() {
            events |= IoEvents::RDHUP | IoEvents::IN;
            if writer_events.contains(IoEvents::ERR) || self.0.writer.is_shutdown() {
                events |= IoEvents::HUP | IoEvents::OUT;
            }
        }

        events |= (reader_events & IoEvents::IN) | (writer_events & IoEvents::OUT);
        events
    }
}

const DAFAULT_BUF_SIZE: usize = 4096;
