// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, collections::btree_set::BTreeSet, sync::Arc, vec::Vec};
use core::{
    borrow::Borrow,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
};

use keyable_arc::KeyableArc;
use ostd::sync::{LocalIrqDisabled, SpinLock, SpinLockGuard};
use smoltcp::{
    iface::Context,
    socket::{tcp::State, udp::UdpMetadata, PollAt},
    time::{Duration, Instant},
    wire::{IpEndpoint, IpRepr, TcpControl, TcpRepr, UdpRepr},
};
use spin::Once;
use takeable::Takeable;

use super::{
    event::{SocketEventObserver, SocketEvents},
    option::{RawTcpOption, RawTcpSetOption},
    unbound::{new_tcp_socket, new_udp_socket},
    RawTcpSocket, RawUdpSocket, TcpStateCheck,
};
use crate::{
    ext::Ext,
    iface::{BindPortConfig, BoundPort, Iface},
};

pub struct Socket<T: Inner<E>, E: Ext>(Takeable<KeyableArc<SocketBg<T, E>>>);

impl<T: Inner<E>, E: Ext> PartialEq for Socket<T, E> {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}
impl<T: Inner<E>, E: Ext> Eq for Socket<T, E> {}
impl<T: Inner<E>, E: Ext> PartialOrd for Socket<T, E> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<T: Inner<E>, E: Ext> Ord for Socket<T, E> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}
impl<T: Inner<E>, E: Ext> Borrow<KeyableArc<SocketBg<T, E>>> for Socket<T, E> {
    fn borrow(&self) -> &KeyableArc<SocketBg<T, E>> {
        self.0.as_ref()
    }
}

/// [`TcpConnectionInner`] or [`UdpSocketInner`].
pub trait Inner<E: Ext> {
    type Observer: SocketEventObserver;

    /// Called by [`Socket::drop`].
    fn on_drop(this: &KeyableArc<SocketBg<Self, E>>)
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

/// States needed by [`TcpConnectionBg`] but not [`UdpSocketBg`].
pub struct TcpConnectionInner<E: Ext> {
    socket: SpinLock<RawTcpSocketExt<E>, LocalIrqDisabled>,
    is_dead: AtomicBool,
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

impl<E: Ext> RawTcpSocketExt<E> {
    fn on_new_state(&mut self, this: &KeyableArc<TcpConnectionBg<E>>) -> SocketEvents {
        if self.may_send() && !self.has_connected {
            self.has_connected = true;

            if let Some(ref listener) = self.listener {
                let mut backlog = listener.inner.lock();
                if let Some(value) = backlog.connecting.take(this) {
                    backlog.connected.push(value);
                }
                listener.add_events(SocketEvents::CAN_RECV);
            }
        }

        self.update_dead(this);

        if self.is_peer_closed() {
            SocketEvents::PEER_CLOSED
        } else if self.is_closed() {
            SocketEvents::CLOSED
        } else {
            SocketEvents::empty()
        }
    }

    /// Updates whether the TCP connection is dead.
    ///
    /// See [`TcpConnectionBg::is_dead`] for the definition of dead TCP connections.
    ///
    /// This method must be called after handling network events. However, it is not necessary to
    /// call this method after handling non-closing user events, because the socket can never be
    /// dead if it is not closed.
    fn update_dead(&self, this: &KeyableArc<TcpConnectionBg<E>>) {
        if self.state() == smoltcp::socket::tcp::State::Closed {
            this.inner.is_dead.store(true, Ordering::Relaxed);
        }

        // According to the current smoltcp implementation, a backlog socket will return back to
        // the `Listen` state if the connection is RSTed before its establishment.
        if self.state() == smoltcp::socket::tcp::State::Listen {
            this.inner.is_dead.store(true, Ordering::Relaxed);

            if let Some(ref listener) = self.listener {
                let mut backlog = listener.inner.lock();
                // This may fail due to race conditions, but it's fine.
                let _ = backlog.connecting.remove(this);
            }
        }
    }
}

impl<E: Ext> TcpConnectionInner<E> {
    fn new(socket: Box<RawTcpSocket>, listener: Option<Arc<TcpListenerBg<E>>>) -> Self {
        let socket_ext = RawTcpSocketExt {
            socket,
            listener,
            has_connected: false,
        };

        TcpConnectionInner {
            socket: SpinLock::new(socket_ext),
            is_dead: AtomicBool::new(false),
        }
    }

    fn lock(&self) -> SpinLockGuard<RawTcpSocketExt<E>, LocalIrqDisabled> {
        self.socket.lock()
    }

    /// Returns whether the TCP connection is dead.
    ///
    /// See [`TcpConnectionBg::is_dead`] for the definition of dead TCP connections.
    fn is_dead(&self) -> bool {
        self.is_dead.load(Ordering::Relaxed)
    }

    /// Sets the TCP connection in [`TimeWait`] state as dead.
    ///
    /// See [`TcpConnectionBg::is_dead`] for the definition of dead TCP connections.
    ///
    /// [`TimeWait`]: smoltcp::socket::tcp::State::TimeWait
    fn set_dead_timewait(&self, socket: &RawTcpSocketExt<E>) {
        debug_assert!(socket.state() == smoltcp::socket::tcp::State::TimeWait);
        self.is_dead.store(true, Ordering::Relaxed);
    }
}

impl<E: Ext> Inner<E> for TcpConnectionInner<E> {
    type Observer = E::TcpEventObserver;

    fn on_drop(this: &KeyableArc<SocketBg<Self, E>>) {
        let mut socket = this.inner.lock();

        // FIXME: Send RSTs when there is unread data.
        socket.close();

        // A TCP connection may not be appropriate for immediate removal. We leave the removal
        // decision to the polling logic.
        this.update_next_poll_at_ms(PollAt::Now);
        socket.update_dead(this);
    }
}

pub struct TcpBacklog<E: Ext> {
    socket: Box<RawTcpSocket>,
    max_conn: usize,
    connecting: BTreeSet<TcpConnection<E>>,
    connected: Vec<TcpConnection<E>>,
}

pub type TcpListenerInner<E> = SpinLock<TcpBacklog<E>, LocalIrqDisabled>;

impl<E: Ext> Inner<E> for TcpListenerInner<E> {
    type Observer = E::TcpEventObserver;

    fn on_drop(this: &KeyableArc<SocketBg<Self, E>>) {
        // A TCP listener can be removed immediately.
        this.bound.iface().common().remove_tcp_listener(this);

        let (connecting, connected) = {
            let mut socket = this.inner.lock();
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

/// States needed by [`UdpSocketBg`] but not [`TcpConnectionBg`].
type UdpSocketInner = SpinLock<Box<RawUdpSocket>, LocalIrqDisabled>;

impl<E: Ext> Inner<E> for UdpSocketInner {
    type Observer = E::UdpEventObserver;

    fn on_drop(this: &KeyableArc<SocketBg<Self, E>>) {
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
        Self(Takeable::new(KeyableArc::new(SocketBg {
            bound,
            inner,
            observer: Once::new(),
            events: AtomicU8::new(0),
            next_poll_at_ms: AtomicU64::new(u64::MAX),
        })))
    }

    pub(crate) fn inner(&self) -> &KeyableArc<SocketBg<T, E>> {
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

#[derive(Debug, Clone, Copy)]
pub struct NeedIfacePoll(bool);

impl NeedIfacePoll {
    pub const TRUE: Self = Self(true);
    pub const FALSE: Self = Self(false);
}

impl Deref for NeedIfacePoll {
    type Target = bool;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
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
    ) -> Result<Self, (BoundPort<E>, smoltcp::socket::tcp::ConnectError)> {
        let socket = {
            let mut socket = new_tcp_socket();

            option.apply(&mut socket);

            let common = bound.iface().common();
            let mut iface = common.interface();

            if let Err(err) = socket.connect(iface.context(), remote_endpoint, bound.port()) {
                drop(iface);
                return Err((bound, err));
            }

            socket
        };

        let inner = TcpConnectionInner::new(socket, None);

        let connection = Self::new(bound, inner);
        connection.0.update_next_poll_at_ms(PollAt::Now);
        connection.init_observer(observer);
        connection
            .iface()
            .common()
            .register_tcp_connection(connection.inner().clone());

        Ok(connection)
    }

    /// Returns the state of the connecting procedure.
    pub fn connect_state(&self) -> ConnectState {
        let socket = self.0.inner.lock();

        if socket.state() == State::SynSent || socket.state() == State::SynReceived {
            ConnectState::Connecting
        } else if socket.has_connected {
            ConnectState::Connected
        } else if KeyableArc::strong_count(self.0.as_ref()) > 1 {
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
        let this: TcpConnectionBg<E> = Arc::into_inner(self.0.take().into())?;
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
    /// Polling the iface is _always_ required after this method succeeds.
    pub fn close(&self) {
        let mut socket = self.0.inner.lock();

        socket.listener = None;
        socket.close();
        self.0.update_next_poll_at_ms(PollAt::Now);
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
    ) -> Result<Self, (BoundPort<E>, smoltcp::socket::tcp::ListenError)> {
        let Some(local_endpoint) = bound.endpoint() else {
            return Err((bound, smoltcp::socket::tcp::ListenError::Unaddressable));
        };

        let socket = {
            let mut socket = new_tcp_socket();

            option.apply(&mut socket);

            if let Err(err) = socket.listen(local_endpoint) {
                return Err((bound, err));
            }

            socket
        };

        let inner = TcpListenerInner::new(TcpBacklog {
            socket,
            max_conn,
            connecting: BTreeSet::new(),
            connected: Vec::new(),
        });

        let listener = Self::new(bound, inner);
        listener.init_observer(observer);
        listener
            .iface()
            .common()
            .register_tcp_listener(listener.inner().clone());

        Ok(listener)
    }

    /// Accepts a TCP connection.
    ///
    /// Polling the iface is _not_ required after this method succeeds.
    pub fn accept(&self) -> Option<(TcpConnection<E>, IpEndpoint)> {
        let accepted = {
            let mut backlog = self.0.inner.lock();
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
        !self.0.inner.lock().connected.is_empty()
    }
}

impl<E: Ext> RawTcpSetOption for TcpListener<E> {
    fn set_keep_alive(&self, interval: Option<Duration>) -> NeedIfacePoll {
        let mut backlog = self.0.inner.lock();
        backlog.socket.set_keep_alive(interval);

        NeedIfacePoll::FALSE
    }

    fn set_nagle_enabled(&self, enabled: bool) {
        let mut backlog = self.0.inner.lock();
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
    ) -> Result<R, crate::errors::udp::SendError>
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        use smoltcp::socket::udp::SendError as SendErrorInner;

        use crate::errors::udp::SendError;

        let mut socket = self.0.inner.lock();

        if size > socket.packet_send_capacity() {
            return Err(SendError::TooLarge);
        }

        let buffer = match socket.send(size, meta) {
            Ok(data) => data,
            Err(SendErrorInner::Unaddressable) => return Err(SendError::Unaddressable),
            Err(SendErrorInner::BufferFull) => return Err(SendError::BufferFull),
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

    pub(crate) fn on_dead_events(this: KeyableArc<Self>)
    where
        T::Observer: Clone,
    {
        // This method can only be called to process network events, so we assume we are holding the
        // poll lock and no race conditions can occur.
        let events = this.events.load(Ordering::Relaxed);
        this.events.store(0, Ordering::Relaxed);

        let observer = this.observer.get().cloned();
        drop(this);

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
                NeedIfacePoll(true)
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
                NeedIfacePoll(false)
            }
        }
    }
}

impl<E: Ext> TcpConnectionBg<E> {
    /// Returns whether the TCP connection is dead.
    ///
    /// A TCP connection is considered dead when and only when the TCP socket is in the closed
    /// state, meaning it's no longer accepting packets from the network. This is different from
    /// the socket file being closed, which only initiates the socket close process.
    pub(crate) fn is_dead(&self) -> bool {
        self.inner.is_dead()
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
        this: &KeyableArc<Self>,
        cx: &mut Context,
        ip_repr: &IpRepr,
        tcp_repr: &TcpRepr,
    ) -> TcpProcessResult {
        let mut socket = this.inner.lock();

        if !socket.accepts(cx, ip_repr, tcp_repr) {
            return TcpProcessResult::NotProcessed;
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
            this.inner.set_dead_timewait(&socket);
            return TcpProcessResult::NotProcessed;
        }

        let old_state = socket.state();
        // For TCP, receiving an ACK packet can free up space in the queue, allowing more packets
        // to be queued.
        let mut events = SocketEvents::CAN_RECV | SocketEvents::CAN_SEND;

        let result = match socket.process(cx, ip_repr, tcp_repr) {
            None => TcpProcessResult::Processed,
            Some((ip_repr, tcp_repr)) => TcpProcessResult::ProcessedWithReply(ip_repr, tcp_repr),
        };

        if socket.state() != old_state {
            events |= socket.on_new_state(this);
        }

        this.add_events(events);
        this.update_next_poll_at_ms(socket.poll_at(cx));

        result
    }

    /// Tries to generate an outgoing packet and dispatches the generated packet.
    pub(crate) fn dispatch<D>(
        this: &KeyableArc<Self>,
        cx: &mut Context,
        dispatch: D,
    ) -> Option<(IpRepr, TcpRepr<'static>)>
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

        if socket.state() != old_state {
            events |= socket.on_new_state(this);
        }

        this.add_events(events);
        this.update_next_poll_at_ms(socket.poll_at(cx));

        reply
    }
}

impl<E: Ext> TcpListenerBg<E> {
    /// Tries to process an incoming packet and returns whether the packet is processed.
    pub(crate) fn process(
        this: &KeyableArc<Self>,
        cx: &mut Context,
        ip_repr: &IpRepr,
        tcp_repr: &TcpRepr,
    ) -> (TcpProcessResult, Option<KeyableArc<TcpConnectionBg<E>>>) {
        let mut backlog = this.inner.lock();

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
            Some(this.clone().into()),
        );
        let conn = TcpConnection::new(
            this.bound
                .iface()
                .bind(BindPortConfig::CanReuse(this.bound.port()))
                .unwrap(),
            inner,
        );
        let conn_bg = conn.inner().clone();

        let inserted = backlog.connecting.insert(conn);
        assert!(inserted);

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
