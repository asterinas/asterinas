// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use aster_virtio::device::socket::{
    header::{VirtioVsockHdr, VirtioVsockOp},
    packet::{TxPacket, TxPacketBuilder},
};

use crate::{
    net::socket::vsock::transport::{
        DEFAULT_RX_BUF_SIZE,
        connection::{ConnectionInner, ConnectionState, TimerState},
    },
    prelude::*,
};

impl ConnectionState {
    pub(super) fn test_and_clear_error(&mut self, conn: &ConnectionInner) -> Result<()> {
        if let Some(error) = self.error.take() {
            conn.pollee.invalidate();
            return Err(error);
        }

        Ok(())
    }

    #[must_use]
    pub(super) fn send_packet(
        &mut self,
        conn: &ConnectionInner,
        op: VirtioVsockOp,
        flags: u32,
    ) -> bool {
        let header = VirtioVsockHdr::new(
            conn.conn_id.local_cid,
            conn.conn_id.peer_cid,
            conn.conn_id.local_port,
            conn.conn_id.peer_port,
            0,
            op,
            flags,
            DEFAULT_RX_BUF_SIZE as u32,
            self.credit.local_fwd_cnt,
        );

        // Lock order: socket state -> device TX

        if conn.bound_port.vsock_space().send_packet(&header) {
            self.credit.last_reported_fwd_cnt = self.credit.local_fwd_cnt;
            true
        } else {
            false
        }
    }

    pub(super) fn make_tx_packet(
        &mut self,
        conn: &ConnectionInner,
        packet_builder: TxPacketBuilder,
    ) -> TxPacket {
        let header = VirtioVsockHdr::new(
            conn.conn_id.local_cid,
            conn.conn_id.peer_cid,
            conn.conn_id.local_port,
            conn.conn_id.peer_port,
            packet_builder.payload_len() as u32,
            VirtioVsockOp::Rw,
            0,
            DEFAULT_RX_BUF_SIZE as u32,
            self.credit.local_fwd_cnt,
        );
        self.credit.last_reported_fwd_cnt = self.credit.local_fwd_cnt;

        packet_builder.build(&header)
    }

    pub(super) fn arm_timeout(&mut self, conn: &ConnectionInner, duration: Duration) {
        use crate::{
            net::socket::vsock::transport::timer::{next_timer_generation, push_timer_event},
            time::{clocks::JIFFIES_TIMER_MANAGER, timer::Timeout},
        };

        let timer_manager = JIFFIES_TIMER_MANAGER.get().unwrap();

        let conn_id = conn.conn_id;
        let generation = next_timer_generation();

        let timer = timer_manager.create_timer(move |_guard| {
            push_timer_event(conn_id, generation);
        });
        timer.lock().set_timeout(Timeout::After(duration));

        self.timer = Some(TimerState { generation, timer });
    }
}
