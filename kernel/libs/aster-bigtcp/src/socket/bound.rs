// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, collections::btree_map::BTreeMap, sync::Arc, vec::Vec};
use core::{
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicU64, AtomicU8, Ordering},
};

use ostd::sync::{LocalIrqDisabled, SpinLock, SpinLockGuard};
use smoltcp::{
    iface::Context,
    socket::{tcp::State, udp::UdpMetadata, PollAt},
    time::{Duration, Instant},
    wire::{IpEndpoint, IpRepr, TcpControl, TcpRepr, UdpRepr},
};
use spin::once::Once;
use takeable::Takeable;

use super::{
    event::{SocketEventObserver, SocketEvents},
    option::{RawTcpOption, RawTcpSetOption},
    unbound::{new_tcp_socket, new_udp_socket},
    RawTcpSocket, RawUdpSocket, TcpStateCheck,
};
use crate::{
    define_boolean_value,
    errors::{
        tcp::{ConnectError, ListenError},
        udp::SendError,
    },
    ext::Ext,
    iface::{BindPortConfig, BoundPort, Iface},
    socket_table::{ConnectionKey, ListenerKey},
};

pub struct Socket<T: Inner<E>, E: Ext>(Takeable<Arc<SocketBg<T, E>>>);

/// [`TcpConnectionInner`], [`TcpListenerInner`], or [`UdpSocketInner`].
pub trait Inner<E: Ext> {
    type Observer: SocketEventObserver;

    /// Called by [`Socket::drop`].
    fn on_drop(this: &Arc<SocketBg<Self, E>>)
    where
        E: Ext,
        Self: Sized;
}

pub type TcpConnection<E> = Socket<TcpConnectionInner<E>, E>;
pub type TcpListener<E> = Socket<TcpListenerInner<E>, E>;
pub type UdpSocket<E> = Socket<UdpSocketInner, E>;

/// Common states shared by [`TcpConnectionBg`] and [`UdpSocketBg`].
///
/// In the type name, `Bg` means "background". Its meaning is described below:
/// - A foreground socket (e.g., [`TcpConnection`]) handles system calls from the user program.
/// - A background socket (e.g., [`TcpConnectionBg`]) handles packets from the network.
pub struct SocketBg<T: Inner<E>, E: Ext> {
    bound: BoundPort<E>,
    inner: T,
    observer: Once<T::Observer>,
    events: AtomicU8,
    next_poll_at_ms: AtomicU64,
}

/// States needed by [`TcpConnectionBg`].
pub struct TcpConnectionInner<E: Ext> {
    socket: SpinLock<RawTcpSocketExt<E>, LocalIrqDisabled>,
    connection_key: ConnectionKey,
}

struct RawTcpSocketExt<E: Ext> {
    socket: Box<RawTcpSocket>,
    listener: Option<Arc<TcpListenerBg<E>>>,
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

define_boolean_value!(
    /// Whether the TCP connection became dead.
    TcpConnBecameDead
);

impl<E: Ext> RawTcpSocketExt<E> {
    fn on_new_state(
        &mut self,
        this: &Arc<TcpConnectionBg<E>>,
    ) -> (SocketEvents, TcpConnBecameDead) {
        if self.may_send() && !self.has_connected {
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

        let events = if self.is_peer_closed() {
            SocketEvents::PEER_CLOSED
        } else if self.is_closed() {
            SocketEvents::CLOSED
        } else {
            SocketEvents::empty()
        };

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
    fn new(socket: Box<RawTcpSocket>, listener: Option<Arc<TcpListenerBg<E>>>) -> Self {
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

    fn lock(&self) -> SpinLockGuard<RawTcpSocketExt<E>, LocalIrqDisabled> {
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

pub struct TcpBacklog<E: Ext> {
    socket: Box<RawTcpSocket>,
    max_conn: usize,
    connecting: BTreeMap<ConnectionKey, TcpConnection<E>>,
    connected: Vec<TcpConnection<E>>,
}

/// States needed by [`TcpListenerBg`].
pub struct TcpListenerInner<E: Ext> {
    backlog: SpinLock<TcpBacklog<E>, LocalIrqDisabled>,
    listener_key: ListenerKey,
}

impl<E: Ext> TcpListenerInner<E> {
    fn new(backlog: TcpBacklog<E>, listener_key: ListenerKey) -> Self {
        Self {
            backlog: SpinLock::new(backlog),
            listener_key,
        }
    }
}

impl<E: Ext> Inner<E> for TcpListenerInner<E> {
    type Observer = E::TcpEventObserver;

    fn on_drop(this: &Arc<SocketBg<Self, E>>) {
        // A TCP listener can be removed immediately.
        this.bound.iface().common().remove_tcp_listener(this);

        let (connecting, connected) = {
            let mut socket = this.inner.backlog.lock();
            (
                core::mem::take(&mut socket.connecting),
                core::mem::take(&mut socket.connected),
            )
        };

        // The lock on `connecting`/`connected` cannot be locked after locking `self`, otherwise we
        // might get a deadlock. due to inconsistent lock order problems.
        //
        // FIXME: Send RSTs instead of going through the normal socket close process.
        drop(connecting);
        drop(connected);
    }
}

/// States needed by [`UdpSocketBg`].
type UdpSocketInner = SpinLock<Box<RawUdpSocket>, LocalIrqDisabled>;

impl<E: Ext> Inner<E> for UdpSocketInner {
    type Observer = E::UdpEventObserver;

    fn on_drop(this: &Arc<SocketBg<Self, E>>) {
        this.inner.lock().close();

        // A UDP socket can be removed immediately.
        this.bound.iface().common().remove_udp_socket(this);
    }
}

impl<T: Inner<E>, E: Ext> Drop for Socket<T, E> {
    fn drop(&mut self) {
        if self.0.is_usable() {
            T::on_drop(&self.0);
        }
    }
}

pub(crate) type TcpConnectionBg<E> = SocketBg<TcpConnectionInner<E>, E>;
pub(crate) type TcpListenerBg<E> = SocketBg<TcpListenerInner<E>, E>;
pub(crate) type UdpSocketBg<E> = SocketBg<UdpSocketInner, E>;

impl<T: Inner<E>, E: Ext> Socket<T, E> {
    pub(crate) fn new(bound: BoundPort<E>, inner: T) -> Self {
        Self(Takeable::new(Arc::new(SocketBg {
            bound,
            inner,
            observer: Once::new(),
            events: AtomicU8::new(0),
            next_poll_at_ms: AtomicU64::new(u64::MAX),
        })))
    }

    pub(crate) fn inner(&self) -> &Arc<SocketBg<T, E>> {
        &self.0
    }
}

impl<T: Inner<E>, E: Ext> Socket<T, E> {
    /// Initializes the observer whose `on_events` will be called when certain iface events happen.
    ///
    /// The caller needs to be responsible for race conditions if network events can occur
    /// simultaneously.
    ///
    /// Calling this method on a socket whose observer has already been initialized will have no
    /// effect.
    pub fn init_observer(&self, new_observer: T::Observer) {
        self.0.observer.call_once(|| new_observer);
    }

    pub fn local_endpoint(&self) -> Option<IpEndpoint> {
        self.0.bound.endpoint()
    }

    pub fn iface(&self) -> &Arc<dyn Iface<E>> {
        self.0.bound.iface()
    }
}

pub enum ConnectState {
    Connecting,
    Connected,
    Refused,
}

define_boolean_value!(
    /// Whether the iface needs to be polled
    NeedIfacePoll
);

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

        if socket.is_closed() {
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
        F: FnOnce(&RawTcpSocket) -> R,
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

impl<E: Ext> TcpListener<E> {
    /// Listens at a specified endpoint.
    ///
    /// Polling the iface is _not_ required after this method succeeds.
    pub fn new_listen(
        bound: BoundPort<E>,
        max_conn: usize,
        option: &RawTcpOption,
        observer: E::TcpEventObserver,
    ) -> Result<Self, (BoundPort<E>, ListenError)> {
        let Some(local_endpoint) = bound.endpoint() else {
            return Err((bound, ListenError::Unaddressable));
        };

        let iface = bound.iface().clone();
        let mut sockets = iface.common().sockets();

        let listener_key = ListenerKey::new(local_endpoint.addr, local_endpoint.port);

        if sockets.lookup_listener(&listener_key).is_some() {
            return Err((bound, ListenError::AddressInUse));
        }

        let socket = {
            let mut socket = new_tcp_socket();

            option.apply(&mut socket);

            if let Err(err) = socket.listen(local_endpoint) {
                return Err((bound, err.into()));
            }

            socket
        };

        let inner = {
            let backlog = TcpBacklog {
                socket,
                max_conn,
                connecting: BTreeMap::new(),
                connected: Vec::new(),
            };

            TcpListenerInner::new(backlog, listener_key)
        };

        let listener = Self::new(bound, inner);
        listener.init_observer(observer);
        let res = sockets.insert_listener(listener.inner().clone());
        debug_assert!(res.is_ok());

        Ok(listener)
    }

    /// Accepts a TCP connection.
    ///
    /// Polling the iface is _not_ required after this method succeeds.
    pub fn accept(&self) -> Option<(TcpConnection<E>, IpEndpoint)> {
        let accepted = {
            let mut backlog = self.0.inner.backlog.lock();
            backlog.connected.pop()?
        };

        let remote_endpoint = {
            // The lock on `accepted` cannot be locked after locking `self`, otherwise we might get
            // a deadlock. due to inconsistent lock order problems.
            let mut socket = accepted.0.inner.lock();

            socket.listener = None;
            socket.remote_endpoint()
        };

        Some((accepted, remote_endpoint.unwrap()))
    }

    /// Returns whether there is a TCP connection to accept.
    ///
    /// It's the caller's responsibility to deal with race conditions when using this method.
    pub fn can_accept(&self) -> bool {
        !self.0.inner.backlog.lock().connected.is_empty()
    }
}

impl<E: Ext> RawTcpSetOption for TcpListener<E> {
    fn set_keep_alive(&self, interval: Option<Duration>) -> NeedIfacePoll {
        let mut backlog = self.0.inner.backlog.lock();
        backlog.socket.set_keep_alive(interval);

        NeedIfacePoll::FALSE
    }

    fn set_nagle_enabled(&self, enabled: bool) {
        let mut backlog = self.0.inner.backlog.lock();
        backlog.socket.set_nagle_enabled(enabled);
    }
}

impl<E: Ext> UdpSocket<E> {
    /// Binds to a specified endpoint.
    ///
    /// Polling the iface is _not_ required after this method succeeds.
    pub fn new_bind(
        bound: BoundPort<E>,
        observer: E::UdpEventObserver,
    ) -> Result<Self, (BoundPort<E>, smoltcp::socket::udp::BindError)> {
        let Some(local_endpoint) = bound.endpoint() else {
            return Err((bound, smoltcp::socket::udp::BindError::Unaddressable));
        };

        let socket = {
            let mut socket = new_udp_socket();

            if let Err(err) = socket.bind(local_endpoint) {
                return Err((bound, err));
            }

            socket
        };

        let inner = UdpSocketInner::new(socket);

        let socket = Self::new(bound, inner);
        socket.init_observer(observer);
        socket
            .iface()
            .common()
            .register_udp_socket(socket.inner().clone());

        Ok(socket)
    }

    /// Sends some data.
    ///
    /// Polling the iface is _always_ required after this method succeeds.
    pub fn send<F, R>(
        &self,
        size: usize,
        meta: impl Into<UdpMetadata>,
        f: F,
    ) -> Result<R, SendError>
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut socket = self.0.inner.lock();

        if size > socket.packet_send_capacity() {
            return Err(SendError::TooLarge);
        }

        let buffer = match socket.send(size, meta) {
            Ok(data) => data,
            Err(err) => return Err(err.into()),
        };
        let result = f(buffer);
        self.0.update_next_poll_at_ms(PollAt::Now);

        Ok(result)
    }

    /// Receives some data.
    ///
    /// Polling the iface is _not_ required after this method succeeds.
    pub fn recv<F, R>(&self, f: F) -> Result<R, smoltcp::socket::udp::RecvError>
    where
        F: FnOnce(&[u8], UdpMetadata) -> R,
    {
        let mut socket = self.0.inner.lock();

        let (data, meta) = socket.recv()?;
        let result = f(data, meta);

        Ok(result)
    }

    /// Calls `f` with an immutable reference to the associated [`RawUdpSocket`].
    //
    // NOTE: If a mutable reference is required, add a method above that correctly updates the next
    // polling time.
    pub fn raw_with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&RawUdpSocket) -> R,
    {
        let socket = self.0.inner.lock();
        f(&socket)
    }
}

impl<T: Inner<E>, E: Ext> SocketBg<T, E> {
    pub(crate) fn has_events(&self) -> bool {
        self.events.load(Ordering::Relaxed) != 0
    }

    pub(crate) fn on_events(&self) {
        // This method can only be called to process network events, so we assume we are holding the
        // poll lock and no race conditions can occur.
        let events = self.events.load(Ordering::Relaxed);
        self.events.store(0, Ordering::Relaxed);

        if let Some(observer) = self.observer.get() {
            observer.on_events(SocketEvents::from_bits_truncate(events));
        }
    }

    pub(crate) fn on_dead_events(self: Arc<Self>)
    where
        T::Observer: Clone,
    {
        // This method can only be called to process network events, so we assume we are holding the
        // poll lock and no race conditions can occur.
        let events = self.events.load(Ordering::Relaxed);
        self.events.store(0, Ordering::Relaxed);

        let observer = self.observer.get().cloned();
        drop(self);

        // Notify dead events after the `Arc` is dropped to ensure the observer sees this event
        // with the expected reference count. See `TcpConnection::connect_state` for an example.
        if let Some(ref observer) = observer {
            observer.on_events(SocketEvents::from_bits_truncate(events));
        }
    }

    fn add_events(&self, new_events: SocketEvents) {
        // This method can only be called to add network events, so we assume we are holding the
        // poll lock and no race conditions can occur.
        let events = self.events.load(Ordering::Relaxed);
        self.events
            .store(events | new_events.bits(), Ordering::Relaxed);
    }

    /// Returns the next polling time.
    ///
    /// Note: a zero means polling should be done now and a `u64::MAX` means no polling is required
    /// before new network or user events.
    pub(crate) fn next_poll_at_ms(&self) -> u64 {
        self.next_poll_at_ms.load(Ordering::Relaxed)
    }

    /// Updates the next polling time according to `poll_at`.
    ///
    /// The update is typically needed after new network or user events have been handled, so this
    /// method also marks that there may be new events, so that the event observer provided by
    /// [`Socket::init_observer`] can be notified later.
    fn update_next_poll_at_ms(&self, poll_at: PollAt) -> NeedIfacePoll {
        match poll_at {
            PollAt::Now => {
                self.next_poll_at_ms.store(0, Ordering::Relaxed);
                NeedIfacePoll::TRUE
            }
            PollAt::Time(instant) => {
                let old_total_millis = self.next_poll_at_ms.load(Ordering::Relaxed);
                let new_total_millis = instant.total_millis() as u64;

                self.next_poll_at_ms
                    .store(new_total_millis, Ordering::Relaxed);

                NeedIfacePoll(new_total_millis < old_total_millis)
            }
            PollAt::Ingress => {
                self.next_poll_at_ms.store(u64::MAX, Ordering::Relaxed);
                NeedIfacePoll::FALSE
            }
        }
    }
}

impl<E: Ext> TcpConnectionBg<E> {
    pub(crate) const fn connection_key(&self) -> &ConnectionKey {
        &self.inner.connection_key
    }
}

impl<E: Ext> TcpListenerBg<E> {
    pub(crate) const fn listener_key(&self) -> &ListenerKey {
        &self.inner.listener_key
    }
}

impl<T: Inner<E>, E: Ext> SocketBg<T, E> {
    /// Returns whether an incoming packet _may_ be processed by the socket.
    ///
    /// The check is intended to be lock-free and fast, but may have false positives.
    pub(crate) fn can_process(&self, dst_port: u16) -> bool {
        self.bound.port() == dst_port
    }

    /// Returns whether the socket _may_ generate an outgoing packet.
    ///
    /// The check is intended to be lock-free and fast, but may have false positives.
    pub(crate) fn need_dispatch(&self, now: Instant) -> bool {
        now.total_millis() as u64 >= self.next_poll_at_ms.load(Ordering::Relaxed)
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

impl<E: Ext> TcpListenerBg<E> {
    /// Tries to process an incoming packet and returns whether the packet is processed.
    pub(crate) fn process(
        self: &Arc<Self>,
        cx: &mut Context,
        ip_repr: &IpRepr,
        tcp_repr: &TcpRepr,
    ) -> (TcpProcessResult, Option<Arc<TcpConnectionBg<E>>>) {
        let mut backlog = self.inner.backlog.lock();

        if !backlog.socket.accepts(cx, ip_repr, tcp_repr) {
            return (TcpProcessResult::NotProcessed, None);
        }

        // FIXME: According to the Linux implementation, `max_conn` is the upper bound of
        // `connected.len()`. We currently limit it to `connected.len() + connecting.len()` for
        // simplicity.
        if backlog.connected.len() + backlog.connecting.len() >= backlog.max_conn {
            return (TcpProcessResult::Processed, None);
        }

        let result = match backlog.socket.process(cx, ip_repr, tcp_repr) {
            None => TcpProcessResult::Processed,
            Some((ip_repr, tcp_repr)) => TcpProcessResult::ProcessedWithReply(ip_repr, tcp_repr),
        };

        if backlog.socket.state() == smoltcp::socket::tcp::State::Listen {
            return (result, None);
        }

        let new_socket = {
            let mut socket = new_tcp_socket();
            RawTcpOption::inherit(&backlog.socket, &mut socket);
            socket.listen(backlog.socket.listen_endpoint()).unwrap();
            socket
        };

        let inner = TcpConnectionInner::new(
            core::mem::replace(&mut backlog.socket, new_socket),
            Some(self.clone()),
        );
        let conn = TcpConnection::new(
            self.bound
                .iface()
                .bind(BindPortConfig::CanReuse(self.bound.port()))
                .unwrap(),
            inner,
        );
        let conn_bg = conn.inner().clone();

        let old_conn = backlog.connecting.insert(*conn_bg.connection_key(), conn);
        debug_assert!(old_conn.is_none());

        conn_bg.update_next_poll_at_ms(PollAt::Now);

        (result, Some(conn_bg))
    }
}

impl<E: Ext> UdpSocketBg<E> {
    /// Tries to process an incoming packet and returns whether the packet is processed.
    pub(crate) fn process(
        &self,
        cx: &mut Context,
        ip_repr: &IpRepr,
        udp_repr: &UdpRepr,
        udp_payload: &[u8],
    ) -> bool {
        let mut socket = self.inner.lock();

        if !socket.accepts(cx, ip_repr, udp_repr) {
            return false;
        }

        socket.process(
            cx,
            smoltcp::phy::PacketMeta::default(),
            ip_repr,
            udp_repr,
            udp_payload,
        );

        self.add_events(SocketEvents::CAN_RECV);
        self.update_next_poll_at_ms(socket.poll_at(cx));

        true
    }

    /// Tries to generate an outgoing packet and dispatches the generated packet.
    pub(crate) fn dispatch<D>(&self, cx: &mut Context, dispatch: D)
    where
        D: FnOnce(&mut Context, &IpRepr, &UdpRepr, &[u8]),
    {
        let mut socket = self.inner.lock();

        socket
            .dispatch(cx, |cx, _meta, (ip_repr, udp_repr, udp_payload)| {
                dispatch(cx, &ip_repr, &udp_repr, udp_payload);
                Ok::<(), ()>(())
            })
            .unwrap();

        // For UDP, dequeuing a packet means that we can queue more packets.
        self.add_events(SocketEvents::CAN_SEND);
        self.update_next_poll_at_ms(socket.poll_at(cx));
    }
}
