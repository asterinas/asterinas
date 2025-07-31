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
        util::{options::SocketOptionSet, ControlMessage, SockShutdownCmd},
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
        options: &SocketOptionSet,
    ) -> (Connected, Connected) {
        let (this_writer, peer_reader) = RingBuffer::new(UNIX_STREAM_DEFAULT_BUF_SIZE).split();
        let (peer_writer, this_reader) = RingBuffer::new(UNIX_STREAM_DEFAULT_BUF_SIZE).split();

        let this_inner = Inner {
            addr: SpinLock::new(addr),
            state,
            is_pass_cred: AtomicBool::new(options.pass_cred()),
            all_aux: Mutex::new(VecDeque::new()),
            has_aux: AtomicBool::new(false),
        };
        let peer_inner = Inner {
            addr: SpinLock::new(peer_addr),
            state: peer_state,
            is_pass_cred: AtomicBool::new(false),
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
        is_seqpacket: bool,
    ) -> Result<(usize, Vec<ControlMessage>)> {
        let is_empty = writer.is_empty();
        if is_empty && !is_seqpacket {
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
        let is_pass_cred = self.inner.this_end().is_pass_cred.load(Ordering::Relaxed);

        // Fast path: There are no auxiliary data to receive.
        if !peer_end.has_aux.load(Ordering::Relaxed) {
            let read_len = self
                .inner
                .read_with(move || reader.read_fallible_with_max_len(writer, no_aux_len))?;
            let ctrl_msgs = if is_pass_cred {
                AuxiliaryData::default().generate_control(is_pass_cred)
            } else {
                Vec::new()
            };
            return Ok((read_len, ctrl_msgs));
        }

        let mut all_aux = peer_end.all_aux.lock();
        let mut aux_prev_data: Option<AuxiliaryData> = None;
        let mut read_tot_len = 0;

        let aux_data = loop {
            let read_start = reader.head();
            let (aux_len, aux_front) = if let Some(front) = all_aux.front_mut() {
                if front.start == read_start {
                    ((front.end - read_start).0, Some(front))
                } else {
                    ((front.start - read_start).0, None)
                }
            } else {
                (usize::MAX, None)
            };

            // Unless the auxiliary data we have already received is a subset of the current
            // auxiliary data, we cannot receive additional bytes.
            if let Some(prev) = aux_prev_data.as_mut() {
                let is_subset = if let Some(front) = aux_front.as_ref() {
                    prev.is_subset_of(&front.data, is_pass_cred)
                } else {
                    prev.is_subset_of(&AuxiliaryData::default(), is_pass_cred)
                };
                if !is_subset {
                    break prev;
                }
            }

            // Read the payload bytes of the current auxiliary data.
            let read_res = if !is_empty && aux_len > 0 {
                self.inner
                    .read_with(|| reader.read_fallible_with_max_len(writer, aux_len))
            } else {
                Ok(0)
            };
            let read_len = match read_res {
                Ok(read_len) => read_len,
                Err(_) if read_tot_len > 0 => break aux_prev_data.as_mut().unwrap(),
                Err(err) => return Err(err),
            };
            read_tot_len += read_len;

            // Record the current auxiliary data. Break if the read is incomplete or this is a
            // `SOCK_SEQPACKET` socket.
            if is_seqpacket {
                aux_prev_data = Some(all_aux.pop_front().unwrap().data);
                if read_len < aux_len {
                    warn!("setting MSG_TRUNC is not supported");
                    reader.skip(aux_len - read_len);
                }
                break aux_prev_data.as_mut().unwrap();
            } else if let Some(front) = aux_front {
                if read_len < aux_len {
                    front.start += read_len;
                    break &mut front.data;
                }
                aux_prev_data = Some(all_aux.pop_front().unwrap().data);
            } else {
                aux_prev_data = Some(AuxiliaryData::default());
                if read_len < aux_len {
                    break aux_prev_data.as_mut().unwrap();
                }
            }
        };

        drop(reader);

        let ctrl_msgs = aux_data.generate_control(is_pass_cred);
        debug_assert!(is_seqpacket || read_tot_len != 0);
        peer_end
            .has_aux
            .store(!all_aux.is_empty(), Ordering::Relaxed);

        Ok((read_tot_len, ctrl_msgs))
    }

    pub(super) fn try_write(
        &self,
        reader: &mut dyn MultiRead,
        aux_data: &mut AuxiliaryData,
        is_seqpacket: bool,
    ) -> Result<usize> {
        let is_empty = reader.is_empty();
        if is_empty {
            if self.inner.is_shutdown() {
                return_errno_with_message!(Errno::EPIPE, "the channel is shut down");
            }
            if !is_seqpacket {
                return Ok(0);
            }
        }

        if is_seqpacket && reader.sum_lens() >= UNIX_STREAM_DEFAULT_BUF_SIZE {
            return_errno_with_message!(Errno::EMSGSIZE, "the message is too large");
        }

        let this_end = self.inner.this_end();
        let need_pass_cred = this_end.is_pass_cred.load(Ordering::Relaxed)
            || self.inner.peer_end().is_pass_cred.load(Ordering::Relaxed);

        // Fast path: There are no auxiliary data to transmit.
        if aux_data.is_empty() && !is_seqpacket && !need_pass_cred {
            let mut writer = self.writer.lock();
            return self.inner.write_with(move || {
                if is_seqpacket && writer.free_len() < reader.sum_lens() {
                    return Ok(0);
                }
                writer.write_fallible(reader)
            });
        }

        let mut all_aux = this_end.all_aux.lock();

        // No matter we succeed later or not, set the flag first to ensure that the auxiliary
        // data are always visible to `try_recv`.
        this_end.has_aux.store(true, Ordering::Relaxed);

        // Write the payload bytes.
        let (write_start, write_res) = if !is_empty {
            let mut writer = self.writer.lock();
            let write_start = writer.tail();
            let write_res = self.inner.write_with(move || {
                if is_seqpacket && writer.free_len() < reader.sum_lens() {
                    return Ok(0);
                }
                writer.write_fallible(reader)
            });
            (write_start, write_res)
        } else {
            (self.writer.lock().tail(), Ok(0))
        };
        let Ok(write_len) = write_res else {
            this_end
                .has_aux
                .store(!all_aux.is_empty(), Ordering::Relaxed);
            return write_res;
        };

        if need_pass_cred {
            aux_data.fill_cred();
        }

        // Store the auxiliary data.
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

    pub(super) fn set_pass_cred(&self, is_pass_cred: bool) {
        self.inner
            .this_end()
            .is_pass_cred
            .store(is_pass_cred, Ordering::Relaxed);
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
    is_pass_cred: AtomicBool,
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
