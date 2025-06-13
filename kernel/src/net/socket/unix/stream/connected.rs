// SPDX-License-Identifier: MPL-2.0

use crate::{
    events::IoEvents,
    fs::utils::{Endpoint, EndpointState},
    net::socket::{
        unix::{addr::UnixSocketAddrBound, UnixSocketAddr},
        util::SockShutdownCmd,
    },
    prelude::*,
    process::signal::Pollee,
    util::{
        ring_buffer::{RbConsumer, RbProducer, RingBuffer},
        MultiRead, MultiWrite,
    },
};

pub(super) struct Connected {
    inner: Endpoint<Inner>,
    reader: Mutex<RbConsumer<u8>>,
    writer: Mutex<RbProducer<u8>>,
}

impl Connected {
    pub(super) fn new_pair(
        addr: Option<UnixSocketAddrBound>,
        peer_addr: Option<UnixSocketAddrBound>,
        state: EndpointState,
        peer_state: EndpointState,
    ) -> (Connected, Connected) {
        let (this_writer, peer_reader) = RingBuffer::new(DEFAULT_BUF_SIZE).split();
        let (peer_writer, this_reader) = RingBuffer::new(DEFAULT_BUF_SIZE).split();

        let this_inner = Inner {
            addr: SpinLock::new(addr),
            state,
        };
        let peer_inner = Inner {
            addr: SpinLock::new(peer_addr),
            state: peer_state,
        };

        let (this_inner, peer_inner) = Endpoint::new_pair(this_inner, peer_inner);

        let this = Connected {
            inner: this_inner,
            reader: Mutex::new(this_reader),
            writer: Mutex::new(this_writer),
        };
        let peer = Connected {
            inner: peer_inner,
            reader: Mutex::new(peer_reader),
            writer: Mutex::new(peer_writer),
        };

        (this, peer)
    }

    pub(super) fn addr(&self) -> Option<UnixSocketAddrBound> {
        self.inner.this_end().addr.lock().clone()
    }

    pub(super) fn peer_addr(&self) -> Option<UnixSocketAddrBound> {
        self.inner.peer_end().addr.lock().clone()
    }

    pub(super) fn bind(&self, addr_to_bind: UnixSocketAddr) -> Result<()> {
        let mut addr = self.inner.this_end().addr.lock();

        if addr.is_some() {
            return addr_to_bind.bind_unnamed();
        }

        let bound_addr = addr_to_bind.bind()?;
        *addr = Some(bound_addr);

        Ok(())
    }

    pub(super) fn try_read(&self, writer: &mut dyn MultiWrite) -> Result<usize> {
        if writer.is_empty() {
            if self.reader.lock().is_empty() {
                return_errno_with_message!(Errno::EAGAIN, "the channel is empty");
            }
            return Ok(0);
        }

        let read = || {
            let mut reader = self.reader.lock();
            reader.read_fallible(writer)
        };

        self.inner.read_with(read)
    }

    pub(super) fn try_write(&self, reader: &mut dyn MultiRead) -> Result<usize> {
        if reader.is_empty() {
            if self.inner.is_shutdown() {
                return_errno_with_message!(Errno::EPIPE, "the channel is shut down");
            }
            return Ok(0);
        }

        let write = || {
            let mut writer = self.writer.lock();
            writer.write_fallible(reader)
        };

        self.inner.write_with(write)
    }

    pub(super) fn shutdown(&self, cmd: SockShutdownCmd) {
        if cmd.shut_read() {
            self.inner.peer_shutdown();
        }

        if cmd.shut_write() {
            self.inner.shutdown();
        }
    }

    pub(super) fn is_read_shutdown(&self) -> bool {
        self.inner.is_peer_shutdown()
    }

    pub(super) fn is_write_shutdown(&self) -> bool {
        self.inner.is_shutdown()
    }

    pub(super) fn check_io_events(&self) -> IoEvents {
        let mut events = IoEvents::empty();

        if !self.reader.lock().is_empty() {
            events |= IoEvents::IN;
        }

        if !self.writer.lock().is_full() {
            events |= IoEvents::OUT;
        }

        events
    }

    pub(super) fn cloned_pollee(&self) -> Pollee {
        self.inner.this_end().state.cloned_pollee()
    }
}

impl Drop for Connected {
    fn drop(&mut self) {
        self.inner.shutdown();
        self.inner.peer_shutdown();
    }
}

struct Inner {
    addr: SpinLock<Option<UnixSocketAddrBound>>,
    state: EndpointState,
}

impl AsRef<EndpointState> for Inner {
    fn as_ref(&self) -> &EndpointState {
        &self.state
    }
}

const DEFAULT_BUF_SIZE: usize = 65536;
