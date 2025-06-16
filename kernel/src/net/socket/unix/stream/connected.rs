// SPDX-License-Identifier: MPL-2.0

use core::{
    num::Wrapping,
    sync::atomic::{AtomicBool, Ordering},
};

use crate::{
    events::IoEvents,
    fs::utils::{ChannelPollee, Peered},
    net::socket::{
        unix::{addr::UnixSocketAddrBound, UnixControlMessage, UnixSocketAddr},
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
    inner: Peered<Inner>,
    reader: Mutex<RbConsumer<u8>>,
    writer: Mutex<RbProducer<u8>>,
}

impl Connected {
    pub(super) fn new_pair(
        addr: Option<UnixSocketAddrBound>,
        peer_addr: Option<UnixSocketAddrBound>,
        is_read_shutdown: bool,
        is_write_shutdown: bool,
        pollee: Pollee,
    ) -> (Connected, Connected) {
        pollee.invalidate();

        let (this_writer, peer_reader) = RingBuffer::new(DEFAULT_BUF_SIZE).split();
        let (peer_writer, this_reader) = RingBuffer::new(DEFAULT_BUF_SIZE).split();

        let this_inner = Inner {
            addr: SpinLock::new(addr),
            pollee: ChannelPollee::with_pollee(pollee, is_write_shutdown),
            all_ctrl_msgs: Mutex::new(VecDeque::new()),
            has_ctrl_msgs: AtomicBool::new(false),
        };
        let peer_inner = Inner {
            addr: SpinLock::new(peer_addr),
            pollee: ChannelPollee::with_pollee(Pollee::new(), is_read_shutdown),
            all_ctrl_msgs: Mutex::new(VecDeque::new()),
            has_ctrl_msgs: AtomicBool::new(false),
        };

        let (this_inner, peer_inner) = Peered::new_pair(this_inner, peer_inner);

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

    pub(super) fn try_read(
        &self,
        writer: &mut dyn MultiWrite,
    ) -> Result<(usize, Vec<UnixControlMessage>)> {
        if writer.is_empty() {
            if self.reader.lock().is_empty() {
                return_errno_with_message!(Errno::EAGAIN, "the channel is empty");
            }
            return Ok((0, Vec::new()));
        }

        let mut reader = self.reader.lock();
        // `reader.len()` is an `Acquire` operation. So it can guarantee that the `has_ctrl_msgs`
        // check below sees the up-to-date value.
        let no_ctrl_len = reader.len();

        let peer_end = self.inner.peer_end();

        // Fast path: There are no control messages to receive.
        if !peer_end.has_ctrl_msgs.load(Ordering::Relaxed) {
            let read_len = self
                .inner
                .read_with(move || reader.read_fallible_with_max_len(writer, no_ctrl_len))?;
            return Ok((read_len, Vec::new()));
        }

        let mut all_ctrl_msgs = peer_end.all_ctrl_msgs.lock();

        let head = reader.head();
        let len_to_ctrl = all_ctrl_msgs
            .front()
            .map(|range| (range.start - head).0)
            .unwrap_or(usize::MAX);
        let len_to_ctrl_end = all_ctrl_msgs
            .front()
            .map(|range| (range.end - head).0)
            .unwrap_or(usize::MAX);

        // It is not allowed to receive two sets of control messages in one `recvmsg`. So we cannot
        // read more than `len_to_ctrl_end` bytes.
        let read_len = self
            .inner
            .read_with(move || reader.read_fallible_with_max_len(writer, len_to_ctrl_end))?;

        if read_len > len_to_ctrl {
            // We have received the first set of control messages.
            let ctrl_msgs = all_ctrl_msgs.pop_front().unwrap().msgs;
            peer_end
                .has_ctrl_msgs
                .store(!all_ctrl_msgs.is_empty(), Ordering::Relaxed);
            Ok((read_len, ctrl_msgs))
        } else {
            Ok((read_len, Vec::new()))
        }
    }

    pub(super) fn try_write(
        &self,
        reader: &mut dyn MultiRead,
        ctrl_msgs: &mut Vec<UnixControlMessage>,
    ) -> Result<usize> {
        if reader.is_empty() {
            if self.inner.is_shutdown() {
                return_errno_with_message!(Errno::EPIPE, "the channel is shut down");
            }
            return Ok(0);
        }

        // Fast path: There are no control messages to transmit.
        if ctrl_msgs.is_empty() {
            let write = || {
                let mut writer = self.writer.lock();
                writer.write_fallible(reader)
            };

            return self.inner.write_with(write);
        }

        let this_end = self.inner.this_end();

        let mut all_ctrl_msgs = this_end.all_ctrl_msgs.lock();

        // No matter we succeed later or not, set the flag first to ensure that the control
        // messages are always visible to `try_recv`.
        this_end.has_ctrl_msgs.store(true, Ordering::Relaxed);

        let mut writer = self.writer.lock();
        let tail = writer.tail();
        let res = self.inner.write_with(move || writer.write_fallible(reader));

        match &res {
            Ok(write_len) => {
                let range = ControlMessageRange {
                    msgs: core::mem::take(ctrl_msgs),
                    start: tail,
                    end: tail + Wrapping(*write_len),
                };
                all_ctrl_msgs.push_back(range);
            }
            Err(_) => {
                this_end
                    .has_ctrl_msgs
                    .store(!all_ctrl_msgs.is_empty(), Ordering::Relaxed);
            }
        }

        res
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
        self.inner.this_end().pollee.cloned_pollee()
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
    pollee: ChannelPollee,
    // Lock order: `reader` -> `all_ctrl_msgs` & `all_ctrl_msgs` -> `writer`
    all_ctrl_msgs: Mutex<VecDeque<ControlMessageRange>>,
    has_ctrl_msgs: AtomicBool,
}

impl AsRef<ChannelPollee> for Inner {
    fn as_ref(&self) -> &ChannelPollee {
        &self.pollee
    }
}

struct ControlMessageRange {
    msgs: Vec<UnixControlMessage>,
    start: Wrapping<usize>, // inclusive
    end: Wrapping<usize>,   // exclusive
}

const DEFAULT_BUF_SIZE: usize = 65536;
