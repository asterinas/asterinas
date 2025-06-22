// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    sync::{Arc, Weak},
};
use core::ops::{Deref, DerefMut};

use aster_softirq::BottomHalfDisabled;
use ostd::sync::{SpinLock, SpinLockGuard};
use smoltcp::{
    socket::{tcp::State, PollAt},
    time::Duration,
    wire::{IpEndpoint, IpRepr, TcpControl, TcpRepr},
};

use super::{
    common::{Inner, NeedIfacePoll, Socket, SocketBg},
    tcp_listen::TcpListenerBg,
};
use crate::{
    define_boolean_value,
    errors::tcp::{ConnectError, RecvError, SendError},
    ext::Ext,
    iface::{BoundPort, PollKey, PollableIfaceMut},
    socket::{
        event::SocketEvents,
        option::{RawTcpOption, RawTcpSetOption},
        unbound::{new_tcp_socket, RawTcpSocket},
    },
    socket_table::ConnectionKey,
};

pub type TcpConnection<E> = Socket<TcpConnectionInner<E>, E>;

/// States needed by [`TcpConnectionBg`].
pub struct TcpConnectionInner<E: Ext> {
    socket: SpinLock<RawTcpSocketExt<E>, BottomHalfDisabled>,
    poll_key: PollKey,
    connection_key: ConnectionKey,
}

pub struct RawTcpSocketExt<E: Ext> {
    socket: Box<RawTcpSocket>,
    pub(super) listener: Option<Arc<TcpListenerBg<E>>>,
    has_connected: bool,
    /// Indicates if the receiving side of this socket is shut down by the user.
    is_recv_shut: bool,
    /// Indicates if the socket is closed by a RST packet.
    is_rst_closed: bool,
}

impl<E: Ext> Deref for RawTcpSocketExt<E> {
    type Target = RawTcpSocket;

    fn deref(&self) -> &Self::Target {
        &self.socket
    }
}

impl<E: Ext> DerefMut for RawTcpSocketExt<E> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.socket
    }
}

impl<E: Ext> RawTcpSocketExt<E> {
    /// Checks if the socket may receive any new data.
    ///
    /// This is similar to [`RawTcpSocket::may_recv`]. However, this method checks if there can be
    /// _new_ data. In other words, if there is already buffered data in the socket,
    /// [`RawTcpSocket::may_recv`] will always return true since it is possible to receive the
    /// buffered data, but this method may return false if the peer has closed its sending half (so
    /// no new data can come in).
    pub fn may_recv_new(&self) -> bool {
        // See also the implementation of `RawTcpSocket::may_recv`.
        match self.state() {
            State::Established => true,
            // Our sending half is closed, but the peer's sending half is still active.
            State::FinWait1 | State::FinWait2 => true,
            _ => false,
        }
    }

    /// Checks if the socket is closing.
    ///
    /// More specifically, we say a socket is closing if and only if it has sent its FIN packet but
    /// is still waiting for an ACK packet from the peer to acknowledge the FIN it sent.
    pub fn is_closing(&self) -> bool {
        let state = self.state();
        matches!(state, State::FinWait1 | State::Closing | State::LastAck)
    }

    /// Returns whether the receiving half of the socket is shut down.
    ///
    /// This method will return true if and only if [`TcpConnection::shut_recv`] or
    /// [`TcpConnection::close`] is called.
    pub fn is_recv_shut(&self) -> bool {
        self.is_recv_shut
    }

    /// Checks if the socket is closed by a RST packet.
    ///
    /// Note that the flag is automatically cleared when it is read by
    /// [`TcpConnection::clear_rst_closed`], [`TcpConnection::send`], or [`TcpConnection::recv`].
    pub fn is_rst_closed(&self) -> bool {
        self.is_rst_closed
    }
}

define_boolean_value!(
    /// Whether the TCP connection became dead.
    TcpConnBecameDead
);

impl<E: Ext> RawTcpSocketExt<E> {
    /// Checks the TCP state for additional events and whether the connection is dead.
    fn check_state(
        &mut self,
        this: &Arc<TcpConnectionBg<E>>,
        old_state: State,
        old_recv_queue: usize,
        is_rst: bool,
    ) -> (SocketEvents, TcpConnBecameDead) {
        let became_dead = if self.state() != State::Established {
            // After the connection is closed by the user, no new data can be read, and such unread
            // data will immediately cause the connection to be reset.
            // Note that "closed" here means that either (1) `close()` or (2) both `shut_send()`
            // and `shut_recv()` are called. In the latter case, there may be some buffered data.
            if self.is_recv_shut
                // These are states where the sending half is closed but new data can come in.
                && matches!(old_state, State::FinWait1 | State::FinWait2)
                && self.recv_queue() > old_recv_queue
            {
                // Strictly speaking, the socket isn't closed by an incoming RST packet in this
                // situation. Instead, we reset the connection and _send_ an outgoing RST packet.
                // However, Linux reports `ECONNRESET`, so we have to follow Linux.
                self.is_rst_closed = true;
                self.abort();
            }
            self.check_dead(this)
        } else {
            TcpConnBecameDead::FALSE
        };

        let events = if self.state() != old_state {
            if self.state() == State::Closed && is_rst {
                self.is_rst_closed = true;
            }
            self.on_new_state(this)
        } else {
            SocketEvents::empty()
        };

        (events, became_dead)
    }

    fn on_new_state(&mut self, this: &Arc<TcpConnectionBg<E>>) -> SocketEvents {
        let may_send = self.may_send();

        if may_send && !self.has_connected {
            self.has_connected = true;

            if let Some(ref listener) = self.listener {
                let mut backlog = listener.inner.backlog.lock();
                if let Some(value) = backlog.connecting.remove(this.connection_key()) {
                    backlog.connected.push(value);
                }
                listener.notify_events(SocketEvents::CAN_RECV);
            }
        }

        let mut events = SocketEvents::empty();
        if !self.may_recv_new() {
            events |= SocketEvents::CLOSED_RECV;
        }
        if !may_send {
            events |= SocketEvents::CLOSED_SEND;
        }

        events
    }

    /// Checks whether the TCP connection becomes dead.
    ///
    /// A TCP connection is considered dead when and only when the TCP socket is in the closed
    /// state, meaning it's no longer accepting packets from the network. This is different from
    /// the socket file being closed, which only initiates the socket close process.
    ///
    /// This method must be called after handling network events. However, it is not necessary to
    /// call this method after handling non-closing user events, because the socket can never be
    /// dead if it is not closed.
    fn check_dead(&self, this: &Arc<TcpConnectionBg<E>>) -> TcpConnBecameDead {
        // According to the current smoltcp implementation, a socket in the CLOSED state with the
        // remote endpoint set means that an outgoing RST packet is pending. We cannot simply mark
        // such a socket as dead.
        if self.state() == State::Closed && self.remote_endpoint().is_none() {
            return TcpConnBecameDead::TRUE;
        }

        // According to the current smoltcp implementation, a backlog socket will return back to
        // the `Listen` state if the connection is RSTed before its establishment.
        if self.state() == State::Listen {
            if let Some(ref listener) = self.listener {
                let mut backlog = listener.inner.backlog.lock();
                // This may fail due to race conditions, but it's fine.
                let _ = backlog.connecting.remove(&this.inner.connection_key);
            }

            return TcpConnBecameDead::TRUE;
        }

        TcpConnBecameDead::FALSE
    }
}

impl<E: Ext> TcpConnectionInner<E> {
    pub(super) fn new(
        socket: Box<RawTcpSocket>,
        listener: Option<Arc<TcpListenerBg<E>>>,
        weak_self: &Weak<TcpConnectionBg<E>>,
    ) -> Self {
        let connection_key = {
            // Since the socket is connected, the following unwrap can never fail
            let local_endpoint = socket.local_endpoint().unwrap();
            let remote_endpoint = socket.remote_endpoint().unwrap();
            ConnectionKey::from((local_endpoint, remote_endpoint))
        };

        let poll_key = PollKey::new(Weak::as_ptr(weak_self).addr());

        let socket_ext = RawTcpSocketExt {
            socket,
            listener,
            has_connected: false,
            is_recv_shut: false,
            is_rst_closed: false,
        };

        TcpConnectionInner {
            socket: SpinLock::new(socket_ext),
            poll_key,
            connection_key,
        }
    }

    pub(super) fn lock(&self) -> SpinLockGuard<RawTcpSocketExt<E>, BottomHalfDisabled> {
        self.socket.lock()
    }
}

impl<E: Ext> Inner<E> for TcpConnectionInner<E> {
    type Observer = E::TcpEventObserver;

    fn on_drop(this: &Arc<SocketBg<Self, E>>) {
        debug_assert!(
            {
                let socket = this.inner.lock();
                if socket.state() == State::Closed {
                    // (1) The socket is fully closed.
                    true
                } else {
                    // (2) The receiving half is closed and the sending half is closing.
                    socket.is_recv_shut
                        && !matches!(
                            socket.state(),
                            State::SynSent
                                | State::SynReceived
                                | State::Established
                                | State::CloseWait,
                        )
                }
            },
            "a connection must be either closed or reset before dropping"
        );
    }
}

pub(crate) type TcpConnectionBg<E> = SocketBg<TcpConnectionInner<E>, E>;

pub enum ConnectState {
    Connecting,
    Connected,
    Refused,
}

impl<E: Ext> TcpConnection<E> {
    /// Connects to a remote endpoint.
    ///
    /// Polling the iface is _always_ required after this method succeeds.
    pub fn new_connect(
        bound: BoundPort<E>,
        remote_endpoint: IpEndpoint,
        option: &RawTcpOption,
        observer: E::TcpEventObserver,
    ) -> Result<Self, (BoundPort<E>, ConnectError)> {
        let Some(local_endpoint) = bound.endpoint() else {
            return Err((bound, ConnectError::Unaddressable));
        };

        let iface = bound.iface().clone();
        // We have to lock `interface` before locking `sockets`
        // to avoid dead lock due to inconsistent lock orders.
        let mut interface = iface.common().interface();
        let mut sockets = iface.common().sockets();

        let connection_key = ConnectionKey::from((local_endpoint, remote_endpoint));

        if sockets.lookup_connection(&connection_key).is_some() {
            return Err((bound, ConnectError::AddressInUse));
        }

        let socket = {
            let mut socket = new_tcp_socket();

            option.apply(&mut socket);

            if let Err(err) = socket.connect(interface.context_mut(), remote_endpoint, bound.port())
            {
                return Err((bound, err.into()));
            }

            socket
        };

        let connection =
            Self::new_cyclic(bound, |weak| TcpConnectionInner::new(socket, None, weak));
        interface.update_next_poll_at_ms(&connection.0, PollAt::Now);
        connection.init_observer(observer);

        let res = sockets.insert_connection(connection.inner().clone());
        debug_assert!(res.is_ok());

        Ok(connection)
    }

    /// Returns the state of the connecting procedure.
    pub fn connect_state(&self) -> ConnectState {
        let socket = self.0.inner.lock();

        if socket.state() == State::SynSent || socket.state() == State::SynReceived {
            ConnectState::Connecting
        } else if socket.has_connected {
            ConnectState::Connected
        } else if Arc::strong_count(self.0.as_ref()) > 1 {
            // Now we should return `ConnectState::Refused`. However, when we do this, we must
            // guarantee that `into_bound_port` can succeed (see the method's doc comments). We can
            // only guarantee this after we have removed all `Arc<TcpConnectionBg>` in the iface's
            // socket set.
            //
            // This branch serves to avoid a race condition: if the removal process hasn't
            // finished, we will return `Connecting` so that the caller won't try to call
            // `into_bound_port` (which may fail immediately).
            ConnectState::Connecting
        } else {
            ConnectState::Refused
        }
    }

    /// Converts back to the [`BoundPort`].
    ///
    /// This method will succeed if the connection is fully closed and no network events can reach
    /// this connection. We guarantee that this method will always succeed if
    /// [`Self::connect_state`] returns [`ConnectState::Refused`].
    pub fn into_bound_port(mut self) -> Option<BoundPort<E>> {
        let this: TcpConnectionBg<E> = Arc::into_inner(self.0.take())?;
        Some(this.bound)
    }

    /// Sends some data.
    ///
    /// Polling the iface _may_ be required after this method succeeds.
    pub fn send<F, R>(&self, f: F) -> Result<(R, NeedIfacePoll), SendError>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        let common = self.iface().common();
        let mut iface = common.interface();

        let mut socket = self.0.inner.lock();

        if socket.is_rst_closed {
            socket.is_rst_closed = false;
            return Err(SendError::ConnReset);
        }
        let result = socket.send(f)?;

        let poll_at = socket.poll_at(iface.context_mut());
        let need_poll = iface.update_next_poll_at_ms(&self.0, poll_at);

        Ok((result, need_poll))
    }

    /// Receives some data.
    ///
    /// Polling the iface _may_ be required after this method succeeds.
    pub fn recv<F, R>(&self, f: F) -> Result<(R, NeedIfacePoll), RecvError>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        let common = self.iface().common();
        let mut iface = common.interface();

        let mut socket = self.0.inner.lock();

        if socket.is_recv_shut && socket.recv_queue() == 0 {
            return Err(RecvError::Finished);
        }
        let result = match socket.recv(f) {
            Err(_) if socket.is_rst_closed => {
                socket.is_rst_closed = false;
                return Err(RecvError::ConnReset);
            }
            res => res,
        }?;

        let poll_at = socket.poll_at(iface.context_mut());
        let need_poll = iface.update_next_poll_at_ms(&self.0, poll_at);

        Ok((result, need_poll))
    }

    /// Checks if the socket is closed by a RST packet and clears the flag.
    ///
    /// This flag is set when the socket is closed by a RST packet, and cleared when the connection
    /// reset error is reported via one of the [`Self::send`], [`Self::recv`], or this method.
    pub fn clear_rst_closed(&self) -> bool {
        let mut socket = self.0.inner.lock();

        let is_rst = socket.is_rst_closed;
        socket.is_rst_closed = false;
        is_rst
    }

    /// Shuts down the sending half of the connection.
    ///
    /// This method will return `false` if the socket is in the CLOSED or TIME_WAIT state.
    ///
    /// Polling the iface is _always_ required after this method succeeds.
    pub fn shut_send(&self) -> bool {
        let mut iface = self.iface().common().interface();
        let mut socket = self.0.inner.lock();

        if matches!(socket.state(), State::Closed | State::TimeWait) {
            return false;
        }

        socket.close();

        let poll_at = socket.poll_at(iface.context_mut());
        iface.update_next_poll_at_ms(&self.0, poll_at);

        true
    }

    /// Shuts down the receiving half of the connection.
    ///
    /// This method will return `false` if the socket is in the CLOSED or TIME_WAIT state.
    ///
    /// Polling the iface is _not_ required after this method succeeds.
    pub fn shut_recv(&self) -> bool {
        let mut socket = self.0.inner.lock();

        if matches!(socket.state(), State::Closed | State::TimeWait) {
            return false;
        }

        socket.is_recv_shut = true;

        true
    }

    /// Closes the connection.
    ///
    /// Polling the iface is _always_ required after this method succeeds.
    ///
    /// Note that either this method or [`Self::reset`] must be called before dropping the TCP
    /// connection to avoid resource leakage.
    pub fn close(&self) {
        let mut iface = self.iface().common().interface();
        let mut socket = self.0.inner.lock();

        socket.is_recv_shut = true;

        if socket.recv_queue() != 0 {
            // If there is unread data, reset the connection immediately.
            socket.abort();
        } else {
            socket.close();
        }

        let poll_at = socket.poll_at(iface.context_mut());
        iface.update_next_poll_at_ms(&self.0, poll_at);
    }

    /// Resets the connection.
    ///
    /// Polling the iface is _always_ required after this method succeeds.
    ///
    /// Note that either this method or [`Self::close`] must be called before dropping the TCP
    /// connection to avoid resource leakage.
    pub fn reset(&self) {
        let mut iface = self.iface().common().interface();
        let mut socket = self.0.inner.lock();

        socket.abort();

        let poll_at = socket.poll_at(iface.context_mut());
        iface.update_next_poll_at_ms(&self.0, poll_at);
    }

    /// Calls `f` with an immutable reference to the associated [`RawTcpSocket`].
    //
    // NOTE: If a mutable reference is required, add a method above that correctly updates the next
    // polling time.
    pub fn raw_with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&RawTcpSocketExt<E>) -> R,
    {
        let socket = self.0.inner.lock();
        f(&socket)
    }
}

impl<E: Ext> RawTcpSetOption for TcpConnection<E> {
    fn set_keep_alive(&self, interval: Option<Duration>) -> NeedIfacePoll {
        let mut iface = self.iface().common().interface();
        let mut socket = self.0.inner.lock();

        socket.set_keep_alive(interval);

        let poll_at = socket.poll_at(iface.context_mut());
        iface.update_next_poll_at_ms(&self.0, poll_at)
    }

    fn set_nagle_enabled(&self, enabled: bool) {
        let mut socket = self.0.inner.lock();
        socket.set_nagle_enabled(enabled);
    }
}

impl<E: Ext> TcpConnectionBg<E> {
    pub(crate) const fn poll_key(&self) -> &PollKey {
        &self.inner.poll_key
    }

    pub(crate) const fn connection_key(&self) -> &ConnectionKey {
        &self.inner.connection_key
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) enum TcpProcessResult {
    NotProcessed,
    Processed,
    ProcessedWithReply(IpRepr, TcpRepr<'static>),
}

impl<E: Ext> TcpConnectionBg<E> {
    /// Tries to process an incoming packet and returns whether the packet is processed.
    pub(crate) fn process(
        self: &Arc<Self>,
        iface: &mut PollableIfaceMut<E>,
        ip_repr: &IpRepr,
        tcp_repr: &TcpRepr,
    ) -> (TcpProcessResult, TcpConnBecameDead) {
        let mut socket = self.inner.lock();

        if !socket.accepts(iface.context_mut(), ip_repr, tcp_repr) {
            return (TcpProcessResult::NotProcessed, TcpConnBecameDead::FALSE);
        }

        // If the socket is in the TIME-WAIT state and a new packet arrives that is a SYN packet
        // without an ACK number, the TIME-WAIT socket will be marked as dead,
        // and the packet will be passed on to any other listening sockets for processing.
        //
        // FIXME: According to the Linux implementation:
        //  * Before marking the socket as dead, we should check some additional fields in the
        //    packet (e.g., the timestamp options) to make sure that the packet is not an old
        //    duplicate packet [1], and ensure that there is actually a listening socket that can
        //    accept the SYN packet [2].
        //  * After marking the socket as dead, we should choose a reasonable initial sequence
        //    number for the newly accepted connection to avoid mixing old duplicate packets with
        //    packets in the new connection [3].
        // All of these detail mechanisms are not currently implemented.
        //
        // [1]: https://elixir.bootlin.com/linux/v6.0.9/source/net/ipv4/tcp_minisocks.c#L211-L214
        // [2]: https://elixir.bootlin.com/linux/v6.0.9/source/net/ipv4/tcp_ipv4.c#L2138-L2145
        // [3]: https://elixir.bootlin.com/linux/v6.0.9/source/net/ipv4/tcp_minisocks.c#L218
        if socket.state() == State::TimeWait
            && tcp_repr.control == TcpControl::Syn
            && tcp_repr.ack_number.is_none()
        {
            // This is a very silly approach to force the socket to go dead. The first `abort` call
            // changes the socket state to `CLOSED`, the second `dispatch` call sets the tuple
            // (local/remote endpoint) to none. So this socket will not accept or send any packets
            // in the future.
            socket.abort();
            socket
                .dispatch(iface.context_mut(), |_, _| {
                    Ok::<(), core::convert::Infallible>(())
                })
                .unwrap();

            iface.update_next_poll_at_ms(self, PollAt::Ingress);
            return (TcpProcessResult::NotProcessed, TcpConnBecameDead::TRUE);
        }

        let old_state = socket.state();
        let old_recv_queue = socket.recv_queue();
        let is_rst = tcp_repr.control == TcpControl::Rst;
        // For TCP, receiving an ACK packet can free up space in the queue, allowing more packets
        // to be queued.
        let mut events = SocketEvents::CAN_RECV | SocketEvents::CAN_SEND;

        let result = match socket.process(iface.context_mut(), ip_repr, tcp_repr) {
            None => TcpProcessResult::Processed,
            Some((ip_repr, tcp_repr)) => TcpProcessResult::ProcessedWithReply(ip_repr, tcp_repr),
        };

        let (state_events, became_dead) =
            socket.check_state(self, old_state, old_recv_queue, is_rst);
        events |= state_events;

        self.notify_events(events);

        let poll_at = socket.poll_at(iface.context_mut());
        iface.update_next_poll_at_ms(self, poll_at);

        (result, became_dead)
    }

    /// Tries to generate an outgoing packet and dispatches the generated packet.
    pub(crate) fn dispatch<D>(
        self: &Arc<Self>,
        iface: &mut PollableIfaceMut<E>,
        dispatch: D,
    ) -> (Option<(IpRepr, TcpRepr<'static>)>, TcpConnBecameDead)
    where
        D: FnOnce(PollableIfaceMut<E>, &IpRepr, &TcpRepr) -> Option<(IpRepr, TcpRepr<'static>)>,
    {
        let mut socket = self.inner.lock();

        let old_state = socket.state();
        let old_recv_queue = socket.recv_queue();
        let mut is_rst = false;
        let mut events = SocketEvents::empty();

        let mut reply = None;
        let (cx, pending) = iface.inner_mut();
        socket
            .dispatch(cx, |cx, (ip_repr, tcp_repr)| {
                reply = dispatch(PollableIfaceMut::new(cx, pending), &ip_repr, &tcp_repr);
                Ok::<(), ()>(())
            })
            .unwrap();

        // `dispatch` can return a packet in response to the generated packet. If the socket
        // accepts the packet, we can process it directly.
        while let Some((ref ip_repr, ref tcp_repr)) = reply {
            if !socket.accepts(iface.context_mut(), ip_repr, tcp_repr) {
                break;
            }
            is_rst |= tcp_repr.control == TcpControl::Rst;
            events |= SocketEvents::CAN_RECV | SocketEvents::CAN_SEND;
            reply = socket.process(iface.context_mut(), ip_repr, tcp_repr);
        }

        let (state_events, became_dead) =
            socket.check_state(self, old_state, old_recv_queue, is_rst);
        events |= state_events;

        self.notify_events(events);

        let poll_at = socket.poll_at(iface.context_mut());
        iface.update_next_poll_at_ms(self, poll_at);

        (reply, became_dead)
    }
}
