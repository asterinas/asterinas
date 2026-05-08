// SPDX-License-Identifier: MPL-2.0

use aster_virtio::device::socket::{header::VirtioVsockOp, packet::RxPacket};

use crate::{
    net::socket::{
        util::SendRecvFlags,
        vsock::transport::{
            CREDIT_UPDATE_THRESHOLD, Connection,
            connection::{ConnectionInner, ConnectionState},
        },
    },
    prelude::*,
    util::MultiWrite,
};

impl Connection {
    /// Copies queued payload bytes into `writer` and updates receive credit accounting.
    pub(in crate::net::socket::vsock) fn try_recv(
        &mut self,
        writer: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        // We use a packet-pool approach here so a receive attempt either completes for the chosen
        // packets or leaves the receive queue unchanged.
        //
        // Otherwise, consider this case: we fully receive some packets, remove them from the
        // queue, and then hit `EFAULT` while receiving the next packet. That would create several
        // problems:
        // - We should not return `EFAULT`, because some bytes have already been received.
        // - We cannot safely return only the bytes from the fully received packets, because
        //   `writer` would then be left at the wrong position.
        // - We also cannot return the bytes from the fully received packets plus the partial bytes
        //   from the faulting packet, because `MultiWrite` does not report how many bytes were
        //   written before `EFAULT` occurred.
        //
        // TODO: Find a better way to report partially written bytes on fault so we can avoid
        // temporarily staging packets just to preserve correct receive semantics.
        let mut packet_pool = [const { None }; 8];

        let Some(mut packets) = self.inner.state.lock().grab_packets_to_recv(
            &self.inner,
            &mut packet_pool[..],
            writer.sum_lens(),
        )?
        else {
            return Ok(0);
        };

        // Packets can only be received from a `&mut connection`. Therefore, releasing the state
        // lock does not cause race conditions. We need to release the lock in order to copy to
        // userspace.
        let receive_behavior = ReceiveBehavior::from_flags(flags);
        let result = packets.receive(receive_behavior, writer);
        let recv_len = *result.as_ref().unwrap_or(&0);

        let mut state = self.inner.state.lock();
        if receive_behavior.is_peek() {
            state.undo_pop_rx_packets(packets);
        } else {
            state.ungrab_packets_and_finish_recv(&self.inner, packets, recv_len);
        }

        self.inner.pollee.invalidate();

        result
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReceiveBehavior {
    Normal,
    Peek,
    Discard,
    PeekDiscard,
}

impl ReceiveBehavior {
    fn from_flags(flags: SendRecvFlags) -> Self {
        match (
            flags.contains(SendRecvFlags::MSG_PEEK),
            flags.contains(SendRecvFlags::MSG_TRUNC),
        ) {
            (true, true) => Self::PeekDiscard,
            (true, false) => Self::Peek,
            (false, true) => Self::Discard,
            (false, false) => Self::Normal,
        }
    }

    fn is_peek(self) -> bool {
        matches!(self, Self::Peek | Self::PeekDiscard)
    }
}

struct PoppedRxPackets<'a> {
    packets: &'a mut [Option<RxPacket>],
    read_offset: usize,
}

impl PoppedRxPackets<'_> {
    fn receive(
        &mut self,
        receive_behavior: ReceiveBehavior,
        writer: &mut dyn MultiWrite,
    ) -> Result<usize> {
        match receive_behavior {
            ReceiveBehavior::Normal => self.copy_to_userspace(writer),
            ReceiveBehavior::Peek => self.copy_to_userspace_peek(writer),
            ReceiveBehavior::Discard => Ok(self.skip_payload(writer.sum_lens())),
            ReceiveBehavior::PeekDiscard => Ok(self.payload_len().min(writer.sum_lens())),
        }
    }

    fn copy_to_userspace(&mut self, writer: &mut dyn MultiWrite) -> Result<usize> {
        let mut read_offset = self.read_offset;
        let mut total_write_len = 0;

        for (i, packet) in self.packets.iter().enumerate() {
            let packet = packet.as_ref().unwrap();

            let mut payload = packet.payload();
            payload.skip(read_offset);

            let write_len = writer.write(&mut payload)?;
            total_write_len += write_len;

            if payload.has_remain() {
                read_offset += write_len;

                self.skip_packets(i);
                self.read_offset = read_offset;
                return Ok(total_write_len);
            }

            read_offset = 0;
        }

        self.packets = &mut [];
        self.read_offset = 0;
        Ok(total_write_len)
    }

    fn copy_to_userspace_peek(&self, writer: &mut dyn MultiWrite) -> Result<usize> {
        let mut read_offset = self.read_offset;
        let mut total_write_len = 0;

        for packet in self.packets.iter() {
            let packet = packet.as_ref().unwrap();

            let mut payload = packet.payload();
            payload.skip(read_offset);

            let write_len = writer.write(&mut payload)?;
            total_write_len += write_len;

            if payload.has_remain() {
                return Ok(total_write_len);
            }

            read_offset = 0;
        }

        Ok(total_write_len)
    }

    fn skip_packets(&mut self, n: usize) {
        let mut packets = &mut [][..];
        core::mem::swap(&mut self.packets, &mut packets);
        packets = &mut packets[n..];
        core::mem::swap(&mut self.packets, &mut packets);
    }

    fn skip_payload(&mut self, mut max_len: usize) -> usize {
        let mut total_len = 0;
        let mut read_offset = self.read_offset;

        for (i, packet) in self.packets.iter().enumerate() {
            let packet = packet.as_ref().unwrap();
            let payload_len = packet.payload_len() - read_offset;
            let skip_len = payload_len.min(max_len);

            total_len += skip_len;
            max_len -= skip_len;

            if skip_len < payload_len {
                read_offset += skip_len;
                self.skip_packets(i);
                self.read_offset = read_offset;
                return total_len;
            }

            if max_len == 0 {
                self.skip_packets(i + 1);
                self.read_offset = 0;
                return total_len;
            }

            read_offset = 0;
        }

        self.packets = &mut [];
        self.read_offset = 0;
        total_len
    }

    fn payload_len(&self) -> usize {
        let mut total_len = 0;
        let mut read_offset = self.read_offset;

        for packet in self.packets.iter() {
            let packet = packet.as_ref().unwrap();
            total_len += packet.payload_len() - read_offset;
            read_offset = 0;
        }

        total_len
    }
}

impl ConnectionState {
    fn grab_packets_to_recv<'a>(
        &mut self,
        conn: &ConnectionInner,
        packet_pool: &'a mut [Option<RxPacket>],
        max_bytes: usize,
    ) -> Result<Option<PoppedRxPackets<'a>>> {
        if max_bytes != 0
            && let Some(packets) = self.pop_rx_packets(&mut packet_pool[..], max_bytes)
        {
            return Ok(Some(packets));
        }

        self.test_and_clear_error(conn)?;

        if max_bytes == 0 || self.shutdown.local_read_closed || self.shutdown.peer_write_closed {
            return Ok(None);
        }

        return_errno_with_message!(Errno::EAGAIN, "the receive buffer is empty");
    }

    fn pop_rx_packets<'a>(
        &mut self,
        packet_pool: &'a mut [Option<RxPacket>],
        mut max_bytes: usize,
    ) -> Option<PoppedRxPackets<'a>> {
        let mut read_offset = None;
        let mut num_packets = 0;

        for packet_opt in packet_pool.iter_mut() {
            *packet_opt = self.rx_queue.packets.pop_front();
            let Some(packet_ref) = packet_opt.as_ref() else {
                break;
            };

            num_packets += 1;

            if read_offset.is_none() {
                read_offset = Some(self.rx_queue.read_offset);
                self.rx_queue.read_offset = 0;
            }

            let payload_len = packet_ref.payload_len();
            if payload_len >= max_bytes {
                break;
            } else {
                max_bytes -= payload_len;
            }
        }

        read_offset.map(|read_offset| PoppedRxPackets {
            packets: &mut packet_pool[0..num_packets],
            read_offset,
        })
    }

    fn ungrab_packets_and_finish_recv(
        &mut self,
        conn: &ConnectionInner,
        packets: PoppedRxPackets,
        recv_len: usize,
    ) {
        self.undo_pop_rx_packets(packets);

        self.rx_queue.used_bytes -= recv_len;
        self.credit.local_fwd_cnt = self.credit.local_fwd_cnt.wrapping_add(recv_len as u32);

        self.send_credit_update_header_if_needed(conn);
    }

    fn undo_pop_rx_packets(&mut self, packets: PoppedRxPackets) {
        debug_assert_eq!(self.rx_queue.read_offset, 0);

        if packets.packets.is_empty() {
            return;
        }

        debug_assert!(packets.read_offset < packets.packets[0].as_ref().unwrap().payload_len());

        for packet_opt in packets.packets.iter_mut().rev() {
            self.rx_queue.packets.push_front(packet_opt.take().unwrap());
        }
        self.rx_queue.read_offset = packets.read_offset;
    }

    fn send_credit_update_header_if_needed(&mut self, conn: &ConnectionInner) {
        let new_credit = self
            .credit
            .local_fwd_cnt
            .wrapping_sub(self.credit.last_reported_fwd_cnt);
        if new_credit < CREDIT_UPDATE_THRESHOLD {
            return;
        }

        // No need to report credit updates if the peer cannot send new data.
        if self.shutdown.peer_write_closed || self.shutdown.local_read_closed {
            return;
        }

        let _ = self.send_packet(conn, VirtioVsockOp::CreditUpdate, 0);
    }
}
