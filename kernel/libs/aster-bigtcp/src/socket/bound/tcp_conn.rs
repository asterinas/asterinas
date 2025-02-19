// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc};
use core::ops::{Deref, DerefMut};

use ostd::sync::{LocalIrqDisabled, SpinLock, SpinLockGuard};
use smoltcp::{
    iface::Context,
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
    errors::tcp::ConnectError,
    ext::Ext,
    iface::BoundPort,
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
    socket: SpinLock<RawTcpSocketExt<E>, LocalIrqDisabled>,
    connection_key: ConnectionKey,
}

pub struct RawTcpSocketExt<E: Ext> {
    socket: Box<RawTcpSocket>,
    pub(super) listener: Option<Arc<TcpListenerBg<E>>>,
    has_connected: bool,
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
}

define_boolean_value!(
    /// Whether the TCP connection became dead.
    TcpConnBecameDead
);

impl<E: Ext> RawTcpSocketExt<E> {
    fn on_new_state(
        &mut self,
        this: &Arc<TcpConnectionBg<E>>,
    ) -> (SocketEvents, TcpConnBecameDead) {
        let may_send = self.may_send();

        if may_send && !self.has_connected {
            self.has_connected = true;

            if let Some(ref listener) = self.listener {
                let mut backlog = listener.inner.backlog.lock();
                if let Some(value) = backlog.connecting.remove(this.connection_key()) {
                    backlog.connected.push(value);
                }
                listener.add_events(SocketEvents::CAN_RECV);
            }
        }

        let became_dead = self.check_dead(this);

        let mut events = SocketEvents::empty();
        if !self.may_recv_new() {
            events |= SocketEvents::CLOSED_RECV;
        }
        if !may_send {
            events |= SocketEvents::CLOSED_SEND;
        }

        (events, became_dead)
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
        // FIXME: This is a temporary workaround to mark TimeWait socket as dead.
        if self.state() == smoltcp::socket::tcp::State::Closed
            || self.state() == smoltcp::socket::tcp::State::TimeWait
        {
            return TcpConnBecameDead::TRUE;
        }

        // According to the current smoltcp implementation, a backlog socket will return back to
        // the `Listen` state if the connection is RSTed before its establishment.
        if self.state() == smoltcp::socket::tcp::State::Listen {
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
    pub(super) fn new(socket: Box<RawTcpSocket>, listener: Option<Arc<TcpListenerBg<E>>>) -> Self {
        let connection_key = {
            // Since the socket is connected, the following unwrap can never fail
            let local_endpoint = socket.local_endpoint().unwrap();
            let remote_endpoint = socket.remote_endpoint().unwrap();
            ConnectionKey::from((local_endpoint, remote_endpoint))
        };

        let socket_ext = RawTcpSocketExt {
            socket,
            listener,
            has_connected: false,
        };

        TcpConnectionInner {
            socket: SpinLock::new(socket_ext),
            connection_key,
        }
    }

    pub(super) fn lock(&self) -> SpinLockGuard<RawTcpSocketExt<E>, LocalIrqDisabled> {
        self.socket.lock()
    }
}

impl<E: Ext> Inner<E> for TcpConnectionInner<E> {
    type Observer = E::TcpEventObserver;

    fn on_drop(this: &Arc<SocketBg<Self, E>>) {
        let became_dead = {
            let mut socket = this.inner.lock();

            // FIXME: Send RSTs when there is unread data.
            socket.close();

            if *socket.check_dead(this) {
                true
            } else {
                // A TCP connection may not be appropriate for immediate removal. We leave the removal
                // decision to the polling logic.
                this.update_next_poll_at_ms(PollAt::Now);
                false
            }
        };

        if became_dead {
            this.bound.iface().common().remove_dead_tcp_connection(this);
        }
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
        // We have to lock interface before locking interface
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

            if let Err(err) = socket.connect(interface.context(), remote_endpoint, bound.port()) {
                return Err((bound, err.into()));
            }

            socket
        };

        let inner = TcpConnectionInner::new(socket, None);

        let connection = Self::new(bound, inner);
        connection.0.update_next_poll_at_ms(PollAt::Now);
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
    pub fn send<F, R>(&self, f: F) -> Result<(R, NeedIfacePoll), smoltcp::socket::tcp::SendError>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        let common = self.iface().common();
        let mut iface = common.interface();

        let mut socket = self.0.inner.lock();

        let result = socket.send(f)?;
        let need_poll = self
            .0
            .update_next_poll_at_ms(socket.poll_at(iface.context()));

        Ok((result, need_poll))
    }

    /// Receives some data.
    ///
    /// Polling the iface _may_ be required after this method succeeds.
    pub fn recv<F, R>(&self, f: F) -> Result<(R, NeedIfacePoll), smoltcp::socket::tcp::RecvError>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        let common = self.iface().common();
        let mut iface = common.interface();

        let mut socket = self.0.inner.lock();

        let result = socket.recv(f)?;
        let need_poll = self
            .0
            .update_next_poll_at_ms(socket.poll_at(iface.context()));

        Ok((result, need_poll))
    }

    /// Closes the connection.
    ///
    /// This method returns `false` if the socket is closed _before_ calling this method.
    ///
    /// Polling the iface is _always_ required after this method succeeds.
    pub fn close(&self) -> bool {
        let mut socket = self.0.inner.lock();

        socket.listener = None;

        if socket.state() == State::Closed {
            return false;
        }

        socket.close();
        self.0.update_next_poll_at_ms(PollAt::Now);

        true
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
        let mut socket = self.0.inner.lock();
        socket.set_keep_alive(interval);

        if interval.is_some() {
            self.0.update_next_poll_at_ms(PollAt::Now);
            NeedIfacePoll::TRUE
        } else {
            NeedIfacePoll::FALSE
        }
    }

    fn set_nagle_enabled(&self, enabled: bool) {
        let mut socket = self.0.inner.lock();
        socket.set_nagle_enabled(enabled);
    }
}

impl<E: Ext> TcpConnectionBg<E> {
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
        cx: &mut Context,
        ip_repr: &IpRepr,
        tcp_repr: &TcpRepr,
    ) -> (TcpProcessResult, TcpConnBecameDead) {
        let mut socket = self.inner.lock();

        if !socket.accepts(cx, ip_repr, tcp_repr) {
            return (TcpProcessResult::NotProcessed, TcpConnBecameDead::FALSE);
        }

        // If the socket is in the TimeWait state and a new packet arrives that is a SYN packet
        // without ack number, the TimeWait socket will be marked as dead,
        // and the packet will be passed on to any other listening sockets for processing.
        //
        // FIXME: Directly marking the TimeWait socket dead is not the correct approach.
        // In Linux, a TimeWait socket remains alive to handle "old duplicate segments".
        // If a TimeWait socket receives a new SYN packet, Linux will select a suitable
        // listening socket from the socket table to respond to that SYN request.
        // (https://elixir.bootlin.com/linux/v6.0.9/source/net/ipv4/tcp_ipv4.c#L2137)
        // Moreover, the Initial Sequence Number (ISN) will be set to prevent the TimeWait socket
        // from erroneously handling packets from the new connection.
        // (https://elixir.bootlin.com/linux/v6.0.9/source/net/ipv4/tcp_minisocks.c#L194)
        // Implementing such behavior is challenging with the current smoltcp APIs.
        if socket.state() == State::TimeWait
            && tcp_repr.control == TcpControl::Syn
            && tcp_repr.ack_number.is_none()
        {
            return (TcpProcessResult::NotProcessed, TcpConnBecameDead::TRUE);
        }

        let old_state = socket.state();
        // For TCP, receiving an ACK packet can free up space in the queue, allowing more packets
        // to be queued.
        let mut events = SocketEvents::CAN_RECV | SocketEvents::CAN_SEND;

        let result = match socket.process(cx, ip_repr, tcp_repr) {
            None => TcpProcessResult::Processed,
            Some((ip_repr, tcp_repr)) => TcpProcessResult::ProcessedWithReply(ip_repr, tcp_repr),
        };

        let became_dead = if socket.state() != old_state {
            let (new_events, became_dead) = socket.on_new_state(self);
            events |= new_events;
            became_dead
        } else {
            TcpConnBecameDead::FALSE
        };

        self.add_events(events);
        self.update_next_poll_at_ms(socket.poll_at(cx));

        (result, became_dead)
    }

    /// Tries to generate an outgoing packet and dispatches the generated packet.
    pub(crate) fn dispatch<D>(
        this: &Arc<Self>,
        cx: &mut Context,
        dispatch: D,
    ) -> (Option<(IpRepr, TcpRepr<'static>)>, TcpConnBecameDead)
    where
        D: FnOnce(&mut Context, &IpRepr, &TcpRepr) -> Option<(IpRepr, TcpRepr<'static>)>,
    {
        let mut socket = this.inner.lock();

        let old_state = socket.state();
        let mut events = SocketEvents::empty();

        let mut reply = None;
        socket
            .dispatch(cx, |cx, (ip_repr, tcp_repr)| {
                reply = dispatch(cx, &ip_repr, &tcp_repr);
                Ok::<(), ()>(())
            })
            .unwrap();

        // `dispatch` can return a packet in response to the generated packet. If the socket
        // accepts the packet, we can process it directly.
        while let Some((ref ip_repr, ref tcp_repr)) = reply {
            if !socket.accepts(cx, ip_repr, tcp_repr) {
                break;
            }
            reply = socket.process(cx, ip_repr, tcp_repr);
            events |= SocketEvents::CAN_RECV | SocketEvents::CAN_SEND;
        }

        let became_dead = if socket.state() != old_state {
            let (new_events, became_dead) = socket.on_new_state(this);
            events |= new_events;
            became_dead
        } else {
            TcpConnBecameDead::FALSE
        };

        this.add_events(events);
        this.update_next_poll_at_ms(socket.poll_at(cx));

        (reply, became_dead)
    }
}
