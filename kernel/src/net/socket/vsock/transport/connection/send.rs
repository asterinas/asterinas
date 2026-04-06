// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use aster_virtio::device::socket::{
    header::VirtioVsockOp,
    packet::{TxPacket, TxPacketBuilder},
    queue::TxCompletion,
};

use crate::{
    events::IoEvents,
    net::socket::{
        util::SendRecvFlags,
        vsock::transport::{
            Connection, DEFAULT_TX_BUF_SIZE,
            connection::{ConnectionInner, ConnectionState},
        },
    },
    prelude::*,
    util::MultiRead,
};

impl Connection {
    /// Copies data from `reader` into TX packets and queues them for transmission.
    ///
    /// The method respects both peer receive credit and the connection's pending-byte budget. It
    /// may return `EAGAIN` when either resource is exhausted.
    pub(in crate::net::socket::vsock) fn try_send(
        &mut self,
        reader: &mut dyn MultiRead,
        _flags: SendRecvFlags,
    ) -> Result<usize> {
        // See the comments in `try_recv` to know why we use a packet-pool approach here.
        let mut packet_pool = [const { None }; 8];

        let num_bytes = self.alloc_send_buffers(&mut packet_pool[..], reader.sum_lens())?;
        if num_bytes == 0 {
            return Ok(0);
        }

        // TODO: If the user sends too many small packets, we'll exhaust a large amount of kernel
        // memory. We need to support merging small packets to avoid this.

        // Packets can only be sending to a `&mut connection`. Therefore, releasing the state
        // lock does not cause race conditions. We need to release the lock in order to copy from
        // userspace.
        Self::copy_to_send_buffers(&mut packet_pool[..], reader, num_bytes)?;

        self.build_and_send_tx_packets(&mut packet_pool[..])?;

        self.inner.pollee.invalidate();

        Ok(num_bytes)
    }

    fn alloc_send_buffers(
        &mut self,
        packet_pool: &mut [Option<TxPacketBuilder>],
        max_bytes: usize,
    ) -> Result<usize> {
        let mut state = self.inner.state.lock();

        state.test_and_clear_error(&self.inner)?;

        if state.shutdown.local_write_closed || state.shutdown.peer_read_closed {
            return_errno_with_message!(Errno::EPIPE, "the connection is closed for writing");
        }

        if max_bytes == 0 {
            return Ok(0);
        }

        let pending_queue_room =
            DEFAULT_TX_BUF_SIZE - self.inner.pending_tx_bytes.load(Ordering::Relaxed);
        if pending_queue_room == 0 {
            return_errno_with_message!(Errno::EAGAIN, "the pending queue is full");
        }

        let credit_room = state.check_peer_credit(&self.inner)?;
        debug_assert_ne!(credit_room, 0);

        let max_bytes = max_bytes.min(pending_queue_room).min(credit_room);
        let mut num_bytes = 0;

        for packet_opt in packet_pool.iter_mut() {
            *packet_opt = Some(TxPacket::new_builder()?);

            num_bytes += TxPacketBuilder::MAX_NBYTES;
            if num_bytes >= max_bytes {
                num_bytes = max_bytes;
                break;
            }
        }

        Ok(num_bytes)
    }

    fn copy_to_send_buffers(
        packet_pool: &mut [Option<TxPacketBuilder>],
        reader: &mut dyn MultiRead,
        num_bytes: usize,
    ) -> Result<()> {
        let mut remaining_bytes = num_bytes;

        for packet_opt in packet_pool.iter_mut() {
            if remaining_bytes == 0 {
                break;
            }

            let packet_mut = packet_opt.as_mut().unwrap();
            let bytes_written = packet_mut.copy_payload(|mut writer| {
                writer.limit(remaining_bytes);
                Ok(reader.read(&mut writer)?)
            })?;
            remaining_bytes -= bytes_written;
        }

        debug_assert_eq!(remaining_bytes, 0);

        Ok(())
    }

    fn build_and_send_tx_packets(&self, packet_pool: &mut [Option<TxPacketBuilder>]) -> Result<()> {
        let mut state = self.inner.state.lock();

        if state.shutdown.local_write_closed || state.shutdown.peer_read_closed {
            return_errno_with_message!(Errno::EPIPE, "the connection is closed for writing");
        }

        let vsock_space = self.inner.bound_port.vsock_space();
        let mut tx = vsock_space.device().lock_tx();

        let mut num_bytes = 0;
        let mut num_bytes_in_pending = 0;

        for packet_opt in packet_pool.iter_mut() {
            let Some(packet_builder) = packet_opt.take() else {
                break;
            };

            let nbytes = packet_builder.payload_len();
            let packet = state.make_tx_packet(&self.inner, packet_builder);

            match tx.try_send(packet) {
                Ok(()) => (),
                Err(pending) => {
                    let completion = ReleasePendingBytes {
                        connection: (*self.inner).clone(),
                        bytes: nbytes,
                    };
                    pending.push_pending(Some(Box::new(completion)));

                    num_bytes_in_pending += nbytes;
                }
            }

            num_bytes += nbytes;
        }

        let old_pending_bytes = self
            .inner
            .pending_tx_bytes
            .fetch_add(num_bytes_in_pending, Ordering::Relaxed);
        debug_assert!(old_pending_bytes + num_bytes_in_pending <= DEFAULT_TX_BUF_SIZE);

        state.consume_peer_credit(num_bytes);

        Ok(())
    }
}

impl ConnectionState {
    fn check_peer_credit(&mut self, conn: &ConnectionInner) -> Result<usize> {
        let peer_free = self.peer_credit();

        if peer_free != 0 {
            return Ok(peer_free);
        }

        if !self.credit.credit_request_pending
            && self.send_packet(conn, VirtioVsockOp::CreditRequest, 0)
        {
            self.credit.credit_request_pending = true;
        }

        return_errno_with_message!(Errno::EAGAIN, "the peer has no receive credit");
    }

    pub(super) fn peer_credit(&self) -> usize {
        let alloc = self.credit.peer_buf_alloc;
        let used = self.credit.tx_cnt.wrapping_sub(self.credit.peer_fwd_cnt);
        alloc.saturating_sub(used) as usize
    }

    fn consume_peer_credit(&mut self, num_bytes: usize) {
        self.credit.tx_cnt = self.credit.tx_cnt.wrapping_add(num_bytes as u32);
    }
}

struct ReleasePendingBytes {
    connection: Arc<ConnectionInner>,
    bytes: usize,
}

impl TxCompletion for ReleasePendingBytes {
    fn on_pending_submit(self: Box<Self>) {
        self.connection
            .pending_tx_bytes
            .fetch_sub(self.bytes, Ordering::Relaxed);
        self.connection.pollee.notify(IoEvents::OUT);
    }
}
