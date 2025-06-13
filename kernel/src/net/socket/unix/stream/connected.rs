// SPDX-License-Identifier: MPL-2.0

use core::{
    num::Wrapping,
    sync::atomic::{AtomicBool, Ordering},
};

use crate::{
    events::IoEvents,
    fs::utils::{Endpoint, EndpointState},
    net::socket::{
        unix::{
            addr::UnixSocketAddrBound, cred::SocketCred, ctrl_msg::AuxiliaryData, UnixSocketAddr,
        },
        util::{ControlMessage, SockShutdownCmd},
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
    peer_cred: SocketCred,
}

impl Connected {
    pub(super) fn new_pair(
        addr: Option<UnixSocketAddrBound>,
        peer_addr: Option<UnixSocketAddrBound>,
        state: EndpointState,
        peer_state: EndpointState,
        cred: SocketCred,
        peer_cred: SocketCred,
    ) -> (Connected, Connected) {
        let (this_writer, peer_reader) = RingBuffer::new(UNIX_STREAM_DEFAULT_BUF_SIZE).split();
        let (peer_writer, this_reader) = RingBuffer::new(UNIX_STREAM_DEFAULT_BUF_SIZE).split();

        let this_inner = Inner {
            addr: SpinLock::new(addr),
            state,
            all_aux: Mutex::new(VecDeque::new()),
            has_aux: AtomicBool::new(false),
        };
        let peer_inner = Inner {
            addr: SpinLock::new(peer_addr),
            state: peer_state,
            all_aux: Mutex::new(VecDeque::new()),
            has_aux: AtomicBool::new(false),
        };

        let (this_inner, peer_inner) = Endpoint::new_pair(this_inner, peer_inner);

        let this = Connected {
            inner: this_inner,
            reader: Mutex::new(this_reader),
            writer: Mutex::new(this_writer),
            peer_cred,
        };
        let peer = Connected {
            inner: peer_inner,
            reader: Mutex::new(peer_reader),
            writer: Mutex::new(peer_writer),
            peer_cred: cred,
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

    pub(super) fn try_read(
        &self,
        writer: &mut dyn MultiWrite,
    ) -> Result<(usize, Vec<ControlMessage>)> {
        if writer.is_empty() {
            if self.reader.lock().is_empty() {
                return_errno_with_message!(Errno::EAGAIN, "the channel is empty");
            }
            return Ok((0, Vec::new()));
        }

        let mut reader = self.reader.lock();
        // `reader.len()` is an `Acquire` operation. So it can guarantee that the `has_aux`
        // check below sees the up-to-date value.
        let no_aux_len = reader.len();

        let peer_end = self.inner.peer_end();

        // Fast path: There are no auxiliary data to receive.
        if !peer_end.has_aux.load(Ordering::Relaxed) {
            let read_len = self
                .inner
                .read_with(move || reader.read_fallible_with_max_len(writer, no_aux_len))?;
            return Ok((read_len, Vec::new()));
        }

        let mut all_aux = peer_end.all_aux.lock();

        let read_start = reader.head();
        let (len_to_aux, len_to_aux_end) = if let Some(front) = all_aux.front() {
            ((front.start - read_start).0, (front.end - read_start).0)
        } else {
            (usize::MAX, usize::MAX)
        };

        // It is not allowed to receive two sets of auxiliary data in one `recvmsg`. So we cannot
        // read more than `len_to_aux_end` bytes.
        let read_len = self
            .inner
            .read_with(move || reader.read_fallible_with_max_len(writer, len_to_aux_end))?;
        if read_len <= len_to_aux {
            return Ok((read_len, Vec::new()));
        }

        // We have received the first set of auxiliary data.
        let ctrl_msgs = all_aux.pop_front().unwrap().data.into_control();
        peer_end
            .has_aux
            .store(!all_aux.is_empty(), Ordering::Relaxed);

        Ok((read_len, ctrl_msgs))
    }

    pub(super) fn try_write(
        &self,
        reader: &mut dyn MultiRead,
        aux_data: &mut AuxiliaryData,
    ) -> Result<usize> {
        if reader.is_empty() {
            if self.inner.is_shutdown() {
                return_errno_with_message!(Errno::EPIPE, "the channel is shut down");
            }
            return Ok(0);
        }

        // Fast path: There are no auxiliary data to transmit.
        if aux_data.is_empty() {
            let mut writer = self.writer.lock();
            return self.inner.write_with(move || writer.write_fallible(reader));
        }

        let this_end = self.inner.this_end();
        let mut all_aux = this_end.all_aux.lock();

        // No matter we succeed later or not, set the flag first to ensure that the auxiliary
        // data are always visible to `try_recv`.
        this_end.has_aux.store(true, Ordering::Relaxed);

        let (write_start, write_res) = {
            let mut writer = self.writer.lock();
            let write_start = writer.tail();
            let write_res = self.inner.write_with(move || writer.write_fallible(reader));
            (write_start, write_res)
        };
        let Ok(write_len) = write_res else {
            this_end
                .has_aux
                .store(!all_aux.is_empty(), Ordering::Relaxed);
            return write_res;
        };

        let aux_range = RangedAuxiliaryData {
            data: core::mem::take(aux_data),
            start: write_start,
            end: write_start + Wrapping(write_len),
        };
        all_aux.push_back(aux_range);

        Ok(write_len)
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

    pub(super) fn peer_cred(&self) -> &SocketCred {
        &self.peer_cred
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
    // Lock order: `reader` -> `all_aux` & `all_aux` -> `writer`
    all_aux: Mutex<VecDeque<RangedAuxiliaryData>>,
    has_aux: AtomicBool,
}

impl AsRef<EndpointState> for Inner {
    fn as_ref(&self) -> &EndpointState {
        &self.state
    }
}

struct RangedAuxiliaryData {
    data: AuxiliaryData,
    start: Wrapping<usize>, // inclusive
    end: Wrapping<usize>,   // exclusive
}

pub(in crate::net) const UNIX_STREAM_DEFAULT_BUF_SIZE: usize = 65536;
