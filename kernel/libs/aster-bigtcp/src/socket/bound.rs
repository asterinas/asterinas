// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    sync::{Arc, Weak},
};
use core::{
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
};

use ostd::sync::{LocalIrqDisabled, RwLock, SpinLock, SpinLockGuard, WriteIrqDisabled};
use smoltcp::{
    iface::Context,
    socket::{tcp::State, udp::UdpMetadata, PollAt},
    time::Instant,
    wire::{IpAddress, IpEndpoint, IpRepr, TcpControl, TcpRepr, UdpRepr},
};

use super::{
    event::{SocketEventObserver, SocketEvents},
    RawTcpSocket, RawUdpSocket, TcpStateCheck,
};
use crate::iface::Iface;

pub struct BoundSocket<T: AnySocket, E>(Arc<BoundSocketInner<T, E>>);

/// [`TcpSocket`] or [`UdpSocket`].
pub trait AnySocket {
    type RawSocket;

    /// Called by [`BoundSocket::new`].
    fn new(socket: Box<Self::RawSocket>) -> Self;

    /// Called by [`BoundSocket::drop`].
    fn on_drop<E>(this: &Arc<BoundSocketInner<Self, E>>)
    where
        Self: Sized;
}

pub type BoundTcpSocket<E> = BoundSocket<TcpSocket, E>;
pub type BoundUdpSocket<E> = BoundSocket<UdpSocket, E>;

/// Common states shared by [`BoundTcpSocketInner`] and [`BoundUdpSocketInner`].
pub struct BoundSocketInner<T, E> {
    iface: Arc<dyn Iface<E>>,
    port: u16,
    socket: T,
    observer: RwLock<Weak<dyn SocketEventObserver>, WriteIrqDisabled>,
    events: AtomicU8,
    next_poll_at_ms: AtomicU64,
}

/// States needed by [`BoundTcpSocketInner`] but not [`BoundUdpSocketInner`].
pub struct TcpSocket {
    socket: SpinLock<RawTcpSocketExt, LocalIrqDisabled>,
    is_dead: AtomicBool,
}

struct RawTcpSocketExt {
    socket: Box<RawTcpSocket>,
    has_connected: bool,
    /// Whether the socket is in the background.
    ///
    /// A background socket is a socket with its corresponding [`BoundSocket`] dropped. This means
    /// that no more user events (like `send`/`recv`) can reach the socket, but it can be in a
    /// state of waiting for certain network events (e.g., remote FIN/ACK packets), so
    /// [`BoundSocketInner`] may still be alive for a while.
    in_background: bool,
}

impl Deref for RawTcpSocketExt {
    type Target = RawTcpSocket;

    fn deref(&self) -> &Self::Target {
        &self.socket
    }
}

impl DerefMut for RawTcpSocketExt {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.socket
    }
}

impl RawTcpSocketExt {
    fn on_new_state(&mut self) -> SocketEvents {
        if self.may_send() {
            self.has_connected = true;
        }

        if self.is_peer_closed() {
            SocketEvents::PEER_CLOSED
        } else if self.is_closed() {
            SocketEvents::CLOSED
        } else {
            SocketEvents::empty()
        }
    }
}

impl TcpSocket {
    fn lock(&self) -> SpinLockGuard<RawTcpSocketExt, LocalIrqDisabled> {
        self.socket.lock()
    }

    /// Returns whether the TCP socket is dead.
    ///
    /// See [`BoundTcpSocketInner::is_dead`] for the definition of dead TCP sockets.
    fn is_dead(&self) -> bool {
        self.is_dead.load(Ordering::Relaxed)
    }

    /// Updates whether the TCP socket is dead.
    ///
    /// See [`BoundTcpSocketInner::is_dead`] for the definition of dead TCP sockets.
    ///
    /// This method must be called after handling network events. However, it is not necessary to
    /// call this method after handling non-closing user events, because the socket can never be
    /// dead if user events can reach the socket.
    fn update_dead(&self, socket: &RawTcpSocketExt) {
        if socket.in_background && socket.state() == smoltcp::socket::tcp::State::Closed {
            self.is_dead.store(true, Ordering::Relaxed);
        }
    }

    /// Sets the TCP socket in [`TimeWait`] state as dead.
    ///
    /// See [`BoundTcpSocketInner::is_dead`] for the definition of dead TCP sockets.
    ///
    /// [`TimeWait`]: smoltcp::socket::tcp::State::TimeWait
    fn set_dead_timewait(&self, socket: &RawTcpSocketExt) {
        debug_assert!(
            socket.in_background && socket.state() == smoltcp::socket::tcp::State::TimeWait
        );
        self.is_dead.store(true, Ordering::Relaxed);
    }
}

impl AnySocket for TcpSocket {
    type RawSocket = RawTcpSocket;

    fn new(socket: Box<Self::RawSocket>) -> Self {
        let socket_ext = RawTcpSocketExt {
            socket,
            has_connected: false,
            in_background: false,
        };

        Self {
            socket: SpinLock::new(socket_ext),
            is_dead: AtomicBool::new(false),
        }
    }

    fn on_drop<E>(this: &Arc<BoundSocketInner<Self, E>>) {
        let mut socket = this.socket.lock();

        socket.in_background = true;
        socket.close();

        // A TCP socket may not be appropriate for immediate removal. We leave the removal decision
        // to the polling logic.
        this.update_next_poll_at_ms(PollAt::Now);
        this.socket.update_dead(&socket);
    }
}

/// States needed by [`BoundUdpSocketInner`] but not [`BoundTcpSocketInner`].
type UdpSocket = SpinLock<Box<RawUdpSocket>, LocalIrqDisabled>;

impl AnySocket for UdpSocket {
    type RawSocket = RawUdpSocket;

    fn new(socket: Box<Self::RawSocket>) -> Self {
        Self::new(socket)
    }

    fn on_drop<E>(this: &Arc<BoundSocketInner<Self, E>>) {
        this.socket.lock().close();

        // A UDP socket can be removed immediately.
        this.iface.common().remove_udp_socket(this);
    }
}

impl<T: AnySocket, E> Drop for BoundSocket<T, E> {
    fn drop(&mut self) {
        T::on_drop(&self.0);
    }
}

pub(crate) type BoundTcpSocketInner<E> = BoundSocketInner<TcpSocket, E>;
pub(crate) type BoundUdpSocketInner<E> = BoundSocketInner<UdpSocket, E>;

impl<T: AnySocket, E> BoundSocket<T, E> {
    pub(crate) fn new(
        iface: Arc<dyn Iface<E>>,
        port: u16,
        socket: Box<T::RawSocket>,
        observer: Weak<dyn SocketEventObserver>,
    ) -> Self {
        Self(Arc::new(BoundSocketInner {
            iface,
            port,
            socket: T::new(socket),
            observer: RwLock::new(observer),
            events: AtomicU8::new(0),
            next_poll_at_ms: AtomicU64::new(u64::MAX),
        }))
    }

    pub(crate) fn inner(&self) -> &Arc<BoundSocketInner<T, E>> {
        &self.0
    }
}

impl<T: AnySocket, E> BoundSocket<T, E> {
    /// Sets the observer whose `on_events` will be called when certain iface events happen. After
    /// setting, the new observer will fire once immediately to avoid missing any events.
    ///
    /// If there is an existing observer, due to race conditions, this function does not guarantee
    /// that the old observer will never be called after the setting. Users should be aware of this
    /// and proactively handle the race conditions if necessary.
    pub fn set_observer(&self, new_observer: Weak<dyn SocketEventObserver>) {
        *self.0.observer.write() = new_observer;

        self.0.on_events();
    }

    /// Returns the observer.
    ///
    /// See also [`Self::set_observer`].
    pub fn observer(&self) -> Weak<dyn SocketEventObserver> {
        self.0.observer.read().clone()
    }

    pub fn local_endpoint(&self) -> Option<IpEndpoint> {
        let ip_addr = {
            let ipv4_addr = self.0.iface.ipv4_addr()?;
            IpAddress::Ipv4(ipv4_addr)
        };
        Some(IpEndpoint::new(ip_addr, self.0.port))
    }

    pub fn iface(&self) -> &Arc<dyn Iface<E>> {
        &self.0.iface
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
    pub const FALSE: Self = Self(false);
}

impl Deref for NeedIfacePoll {
    type Target = bool;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<E> BoundTcpSocket<E> {
    /// Connects to a remote endpoint.
    ///
    /// Polling the iface is _always_ required after this method succeeds.
    pub fn connect(
        &self,
        remote_endpoint: IpEndpoint,
    ) -> Result<(), smoltcp::socket::tcp::ConnectError> {
        let common = self.iface().common();
        let mut iface = common.interface();

        let mut socket = self.0.socket.lock();

        socket.connect(iface.context(), remote_endpoint, self.0.port)?;

        socket.has_connected = false;
        self.0.update_next_poll_at_ms(PollAt::Now);

        Ok(())
    }

    /// Returns the state of the connecting procedure.
    pub fn connect_state(&self) -> ConnectState {
        let socket = self.0.socket.lock();

        if socket.state() == State::SynSent || socket.state() == State::SynReceived {
            ConnectState::Connecting
        } else if socket.has_connected {
            ConnectState::Connected
        } else {
            ConnectState::Refused
        }
    }

    /// Listens at a specified endpoint.
    ///
    /// Polling the iface is _not_ required after this method succeeds.
    pub fn listen(
        &self,
        local_endpoint: IpEndpoint,
    ) -> Result<(), smoltcp::socket::tcp::ListenError> {
        let mut socket = self.0.socket.lock();

        socket.listen(local_endpoint)
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

        let mut socket = self.0.socket.lock();

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

        let mut socket = self.0.socket.lock();

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
        let mut socket = self.0.socket.lock();

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
        let socket = self.0.socket.lock();
        f(&socket)
    }
}

impl<E> BoundUdpSocket<E> {
    /// Binds to a specified endpoint.
    ///
    /// Polling the iface is _not_ required after this method succeeds.
    pub fn bind(&self, local_endpoint: IpEndpoint) -> Result<(), smoltcp::socket::udp::BindError> {
        let mut socket = self.0.socket.lock();

        socket.bind(local_endpoint)
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

        let mut socket = self.0.socket.lock();

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
        let mut socket = self.0.socket.lock();

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
        let socket = self.0.socket.lock();
        f(&socket)
    }
}

impl<T, E> BoundSocketInner<T, E> {
    pub(crate) fn has_events(&self) -> bool {
        self.events.load(Ordering::Relaxed) != 0
    }

    pub(crate) fn on_events(&self) {
        // This method can only be called to process network events, so we assume we are holding the
        // poll lock and no race conditions can occur.
        let events = self.events.load(Ordering::Relaxed);
        self.events.store(0, Ordering::Relaxed);

        // We never hold the write lock in IRQ handlers, so we don't need to disable IRQs when we
        // get the read lock.
        let observer = Weak::upgrade(&*self.observer.read());

        if let Some(inner) = observer {
            inner.on_events(SocketEvents::from_bits_truncate(events));
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
    /// [`BoundSocket::set_observer`] can be notified later.
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

impl<T, E> BoundSocketInner<T, E> {
    pub(crate) fn port(&self) -> u16 {
        self.port
    }
}

impl<E> BoundTcpSocketInner<E> {
    /// Returns whether the TCP socket is dead.
    ///
    /// A TCP socket is considered dead if and only if the following two conditions are met:
    /// 1. The TCP connection is closed, so this socket cannot process any network events.
    /// 2. The socket handle [`BoundTcpSocket`] is dropped, which means that this
    ///    [`BoundSocketInner`] is in background and no more user events can reach it.
    pub(crate) fn is_dead(&self) -> bool {
        self.socket.is_dead()
    }
}

impl<T, E> BoundSocketInner<T, E> {
    /// Returns whether an incoming packet _may_ be processed by the socket.
    ///
    /// The check is intended to be lock-free and fast, but may have false positives.
    pub(crate) fn can_process(&self, dst_port: u16) -> bool {
        self.port == dst_port
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

impl<E> BoundTcpSocketInner<E> {
    /// Tries to process an incoming packet and returns whether the packet is processed.
    pub(crate) fn process(
        &self,
        cx: &mut Context,
        ip_repr: &IpRepr,
        tcp_repr: &TcpRepr,
    ) -> TcpProcessResult {
        let mut socket = self.socket.lock();

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
            self.socket.set_dead_timewait(&socket);
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
            events |= socket.on_new_state();
        }

        self.add_events(events);
        self.update_next_poll_at_ms(socket.poll_at(cx));
        self.socket.update_dead(&socket);

        result
    }

    /// Tries to generate an outgoing packet and dispatches the generated packet.
    pub(crate) fn dispatch<D>(
        &self,
        cx: &mut Context,
        dispatch: D,
    ) -> Option<(IpRepr, TcpRepr<'static>)>
    where
        D: FnOnce(&mut Context, &IpRepr, &TcpRepr) -> Option<(IpRepr, TcpRepr<'static>)>,
    {
        let mut socket = self.socket.lock();

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
            events |= socket.on_new_state();
        }

        self.add_events(events);
        self.update_next_poll_at_ms(socket.poll_at(cx));
        self.socket.update_dead(&socket);

        reply
    }
}

impl<E> BoundUdpSocketInner<E> {
    /// Tries to process an incoming packet and returns whether the packet is processed.
    pub(crate) fn process(
        &self,
        cx: &mut Context,
        ip_repr: &IpRepr,
        udp_repr: &UdpRepr,
        udp_payload: &[u8],
    ) -> bool {
        let mut socket = self.socket.lock();

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
        let mut socket = self.socket.lock();

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
