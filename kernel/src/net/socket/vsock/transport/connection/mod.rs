// SPDX-License-Identifier: MPL-2.0

// Implementation notes:
//
// This file implements handling of incoming packets, so those interfaces are kept `pub(super)`.
//
// Interfaces exposed to the socket layer (`pub(in crate::net::socket::vsock)`) are split into
// submodules by responsibility:
// - `basic.rs` exposes address, error, and poll-state queries.
// - `connect.rs` exposes the connect-handshake result interface.
// - `send.rs` handles transmit buffering, pending-byte accounting, and credit consumption.
// - `recv.rs` handles receive buffering and credit reporting.
// - `shutdown.rs` handles half-close, close-on-drop, and the closing timeout.
// - `utils.rs` contains helpers shared by one or more submodules above.

mod basic;
pub(super) mod connect;
mod recv;
mod send;
mod shutdown;
mod utils;

use core::sync::atomic::AtomicUsize;

use aster_softirq::BottomHalfDisabled;
use aster_virtio::device::socket::{
    header::{VirtioVsockHdr, VirtioVsockOp, VirtioVsockShutdownFlags},
    packet::RxPacket,
};
use ostd::sync::SpinLock;
use takeable::Takeable;

use crate::{
    events::IoEvents,
    net::socket::vsock::transport::{
        BoundPort, DEFAULT_CONNECT_TIMEOUT, DEFAULT_RX_BUF_SIZE, conn_id::ConnId,
    },
    prelude::*,
    process::signal::Pollee,
    time::Timer,
};

/// A uniquely owned vsock connection handle; dropping it will close the connection.
pub(in crate::net::socket::vsock) struct Connection {
    inner: Takeable<Arc<ConnectionInner>>,
}

impl Connection {
    pub(super) fn new(inner: Arc<ConnectionInner>) -> Self {
        Self {
            inner: Takeable::new(inner),
        }
    }
}

pub(super) struct ConnectionInner {
    conn_id: ConnId,
    bound_port: BoundPort,
    pollee: Pollee,
    state: SpinLock<ConnectionState, BottomHalfDisabled>,
    /// The number of TX bytes that have already been queued in the device's pending queue.
    ///
    /// See [`TxQueue`] for more information on the pending queue. This counter is used for
    /// accounting purposes to ensure that the total number of bytes does not exceed the upper
    /// bound, [`DEFAULT_TX_BUF_SIZE`].
    ///
    /// [`TxQueue`]: aster_virtio::device::socket::queue::TxQueue
    /// [`DEFAULT_TX_BUF_SIZE`]: super::DEFAULT_TX_BUF_SIZE
    pending_tx_bytes: AtomicUsize,
}

struct ConnectionState {
    phase: Phase,
    error: Option<Error>,
    rx_queue: RxQueue,
    credit: CreditState,
    shutdown: ShutdownState,
    /// Tracks the deadline for leaving [`Phase::Connecting`] or [`Phase::Closing`].
    ///
    /// INVARIANT: This is `Some(_)` if and only if the phase is
    /// [`Phase::Connecting`] or [`Phase::Closing`].
    timer: Option<TimerState>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    /// Represents the initial state of a newly created connection.
    ///
    /// INVARIANT: In this state, the peer endpoint is fully closed; that is,
    /// both `peer_write_closed` and `peer_read_closed` are `true`.
    Connecting,

    /// Represents the state reached from `Connecting` when the connection
    /// request is rejected or times out.
    ///
    /// INVARIANT: In this state, the peer endpoint is fully closed.
    ConnectFailed,

    /// Represents the initial state of an accepted connection, or the state
    /// reached from `Connecting` when the connection request succeeds.
    ///
    /// INVARIANT: In this state, the peer endpoint is _NOT_ fully closed.
    Connected,

    /// Represents the state reached from `Connected` when the socket is closed
    /// locally, but the peer has not reset the connection.
    ///
    /// INVARIANT: In this state, the local endpoint is fully closed; that is,
    /// both `local_write_closed` and `local_read_closed` are `true`. The peer
    /// endpoint is _NOT_ fully closed.
    Closing,

    /// Represents the state reached from `Connected` or `Closing` when the peer
    /// fully shuts down the connection or resets it, or when a close timeout
    /// expires in `Closing`.
    ///
    /// INVARIANT: In this state, the peer endpoint is fully closed.
    Closed,
}

struct RxQueue {
    packets: VecDeque<RxPacket>,
    used_bytes: usize,
    read_offset: usize,
}

struct CreditState {
    peer_buf_alloc: u32,
    peer_fwd_cnt: u32,
    local_fwd_cnt: u32,
    last_reported_fwd_cnt: u32,
    credit_request_pending: bool,
    tx_cnt: u32,
}

struct ShutdownState {
    local_read_closed: bool,
    local_write_closed: bool,
    peer_read_closed: bool,
    peer_write_closed: bool,
}

struct TimerState {
    generation: u64,
    #[expect(dead_code)]
    timer: Arc<Timer>,
}

impl ConnectionInner {
    pub(super) fn new_connecting(
        bound_port: BoundPort,
        conn_id: &ConnId,
        pollee: Pollee,
    ) -> Arc<Self> {
        pollee.invalidate();

        let this = Self::new(bound_port, conn_id, pollee, Phase::Connecting);

        let mut state = this.state.lock();
        let _ = state.send_packet(&this, VirtioVsockOp::Request, 0);
        state.arm_timeout(&this, DEFAULT_CONNECT_TIMEOUT);
        drop(state);

        this
    }

    pub(super) fn new_connected(
        bound_port: BoundPort,
        conn_id: &ConnId,
        header: &VirtioVsockHdr,
    ) -> Arc<Self> {
        let this = Self::new(bound_port, conn_id, Pollee::new(), Phase::Connected);

        let mut state = this.state.lock();
        state.update_peer_credit(&this, header);
        let _ = state.send_packet(&this, VirtioVsockOp::Response, 0);
        drop(state);

        this
    }

    fn new(bound_port: BoundPort, conn_id: &ConnId, pollee: Pollee, phase: Phase) -> Arc<Self> {
        debug_assert_eq!(bound_port.port(), conn_id.local_port);

        let peer_fully_closed = phase != Phase::Connected;

        let state = ConnectionState {
            phase,
            error: None,
            rx_queue: RxQueue {
                packets: VecDeque::new(),
                used_bytes: 0,
                read_offset: 0,
            },
            credit: CreditState {
                peer_buf_alloc: 0,
                peer_fwd_cnt: 0,
                local_fwd_cnt: 0,
                last_reported_fwd_cnt: 0,
                credit_request_pending: false,
                tx_cnt: 0,
            },
            shutdown: ShutdownState {
                local_read_closed: false,
                local_write_closed: false,
                peer_read_closed: peer_fully_closed,
                peer_write_closed: peer_fully_closed,
            },
            timer: None,
        };

        Arc::new(Self {
            conn_id: *conn_id,
            bound_port,
            pollee,
            state: SpinLock::new(state),
            pending_tx_bytes: AtomicUsize::new(0),
        })
    }

    pub(super) const fn conn_id(&self) -> ConnId {
        self.conn_id
    }

    pub(super) fn pollee(&self) -> &Pollee {
        &self.pollee
    }

    pub(super) fn on_response(&self, header: &VirtioVsockHdr) -> Result<()> {
        let mut state = self.state.lock();

        if state.phase != Phase::Connecting {
            state.active_rst(self);
            return_errno_with_message!(Errno::EISCONN, "the connection is established");
        }

        state.update_peer_credit(self, header);

        state.phase = Phase::Connected;
        state.shutdown.peer_read_closed = false;
        state.shutdown.peer_write_closed = false;
        state.timer = None;

        drop(state);
        self.pollee.notify(IoEvents::OUT);

        Ok(())
    }

    pub(super) fn on_rst(&self) {
        let mut state = self.state.lock();

        state.do_rst(false);

        // The caller will notify the pollee _after_ removing the connection from the table.
    }

    pub(super) fn on_shutdown(&self, header: &VirtioVsockHdr) -> bool {
        let mut state = self.state.lock();
        let mut notify_events = IoEvents::empty();

        if state.phase == Phase::Connecting {
            state.active_rst(self);
            return true;
        }

        let flags = VirtioVsockShutdownFlags::from_bits_truncate(header.flags);
        if flags.contains(VirtioVsockShutdownFlags::SEND) && !state.shutdown.peer_write_closed {
            state.shutdown.peer_write_closed = true;
            notify_events |= IoEvents::IN | IoEvents::OUT | IoEvents::RDHUP | IoEvents::HUP;
        }
        if flags.contains(VirtioVsockShutdownFlags::RECEIVE) && !state.shutdown.peer_read_closed {
            state.shutdown.peer_read_closed = true;
            notify_events |= IoEvents::OUT;
        }

        if notify_events.is_empty() {
            return false;
        }

        // Remove the connection from the table once the peer has fully shut down. See the Linux
        // commit for the reason:
        // <https://github.com/torvalds/linux/commit/3a5cc90a4d1756072619fe511d07621bdef7f120>
        let peer_fully_closed = state.shutdown.peer_read_closed && state.shutdown.peer_write_closed;
        let should_remove = if peer_fully_closed {
            state.phase = Phase::Closed;
            let _ = state.send_packet(self, VirtioVsockOp::Rst, 0);
            true
        } else {
            false
        };

        drop(state);
        self.pollee.notify(notify_events);

        should_remove
    }

    pub(super) fn on_rw(&self, header: &VirtioVsockHdr, packet: RxPacket) -> Result<()> {
        let mut state = self.state.lock();

        if state.shutdown.peer_write_closed {
            // We don't check `local_read_closed` because the peer cannot immediately know this
            // information.
            state.active_rst(self);
            return_errno_with_message!(Errno::ENOTCONN, "the connection is not established");
        }

        let len = packet.payload_len();
        if state.rx_queue.used_bytes + len > DEFAULT_RX_BUF_SIZE {
            state.active_rst(self);
            return_errno_with_message!(Errno::ENOMEM, "the receive queue is full");
        }

        state.update_peer_credit(self, header);

        if len != 0 {
            state.rx_queue.used_bytes += len;
            state.rx_queue.packets.push_back(packet);
        }

        // TODO: If the peer sends too many small packets, we'll exhaust a large amount of kernel
        // memory. We need to support merging small packets to avoid this.

        drop(state);
        self.pollee.notify(IoEvents::IN);

        Ok(())
    }

    pub(super) fn on_credit_update(&self, header: &VirtioVsockHdr) -> Result<()> {
        let mut state = self.state.lock();

        if state.phase == Phase::Connecting {
            state.active_rst(self);
            return_errno_with_message!(Errno::ENOTCONN, "the connection is not established");
        }

        state.update_peer_credit(self, header);

        state.credit.credit_request_pending = false;

        Ok(())
    }

    pub(super) fn on_credit_request(&self, header: &VirtioVsockHdr) -> Result<()> {
        let mut state = self.state.lock();

        if state.phase == Phase::Connecting {
            state.active_rst(self);
            return_errno_with_message!(Errno::ENOTCONN, "the connection is not established");
        }

        state.update_peer_credit(self, header);

        let _ = state.send_packet(self, VirtioVsockOp::CreditUpdate, 0);

        Ok(())
    }

    pub(super) fn on_timeout(&self, generation: u64) -> bool {
        let mut state = self.state.lock();

        let Some(timer) = state.timer.as_ref() else {
            return false;
        };
        if timer.generation != generation {
            return false;
        }

        state.active_rst(self);

        // If the connection resets before this method is reached, the timer will already be set to
        // `None`, so we won't get here. Therefore, we know that the connection timed out.
        if state.phase == Phase::ConnectFailed {
            state.error = Some(Error::with_message(
                Errno::ETIMEDOUT,
                "the connection timed out",
            ));
        }

        true
    }

    pub(super) fn active_rst(&self) {
        let mut state = self.state.lock();

        state.active_rst(self);
    }
}

impl ConnectionState {
    fn active_rst(&mut self, conn: &ConnectionInner) {
        if self.do_rst(true) {
            let _ = self.send_packet(conn, VirtioVsockOp::Rst, 0);
        }

        // The caller will notify the pollee _after_ removing the connection from the table.
    }

    fn do_rst(&mut self, is_active: bool) -> bool {
        match self.phase {
            Phase::Connecting => {
                self.phase = Phase::ConnectFailed;
                self.error = Some(Error::with_message(
                    Errno::ECONNRESET,
                    "the connection is refused",
                ));
                self.timer = None;

                true
            }
            Phase::Connected => {
                self.phase = Phase::Closed;
                // Even though it seems like the most suitable error, Linux never reports
                // `ECONNRESET` when an RST packet is received in the connected state. We follow
                // Linux behavior.
                //
                // Note that Linux does report `ECONNRESET` in the active path (e.g., when the CID
                // changes).
                //
                // FIXME: Our approach is more aggressive because we report `ECONNRESET` for many
                // protocol errors. In contrast, Linux only reports `EPROTO` in some cases and does
                // not necessarily cause the connection to be reset.
                self.error = is_active.then_some(Error::with_message(
                    Errno::ECONNRESET,
                    "the connection is reset",
                ));
                self.shutdown.peer_read_closed = true;
                self.shutdown.peer_write_closed = true;

                true
            }
            Phase::Closing => {
                self.phase = Phase::Closed;
                self.shutdown.peer_read_closed = true;
                self.shutdown.peer_write_closed = true;
                self.timer = None;

                true
            }

            Phase::ConnectFailed | Phase::Closed => false,
        }
    }

    fn update_peer_credit(&mut self, conn: &ConnectionInner, header: &VirtioVsockHdr) {
        let mut should_notify = false;

        // If the peer shrinks its advertised allocation, we still need to invalidate the `Pollee`,
        // so we include that case as well. False positives from `Pollee::notify` are acceptable.
        should_notify |= self.credit.peer_buf_alloc != header.buf_alloc;
        self.credit.peer_buf_alloc = header.buf_alloc;

        should_notify |= self.credit.peer_fwd_cnt != header.fwd_cnt;
        self.credit.peer_fwd_cnt = header.fwd_cnt;

        if should_notify {
            conn.pollee.notify(IoEvents::OUT);
        }
    }
}
