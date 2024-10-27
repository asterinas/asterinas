// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    sync::{Arc, Weak},
};
use core::{
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use ostd::sync::{LocalIrqDisabled, RwLock, SpinLock};
use smoltcp::{
    iface::Context,
    socket::{udp::UdpMetadata, PollAt},
    time::Instant,
    wire::{IpAddress, IpEndpoint, IpRepr, TcpRepr, UdpRepr},
};

use super::{event::SocketEventObserver, RawTcpSocket, RawUdpSocket};
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
    observer: RwLock<Weak<dyn SocketEventObserver>>,
    next_poll_at_ms: AtomicU64,
    has_new_events: AtomicBool,
}

/// States needed by [`BoundTcpSocketInner`] but not [`BoundUdpSocketInner`].
pub struct TcpSocket {
    socket: SpinLock<RawTcpSocketExt, LocalIrqDisabled>,
    is_dead: AtomicBool,
}

struct RawTcpSocketExt {
    socket: Box<RawTcpSocket>,
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

impl TcpSocket {
    fn lock_with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut RawTcpSocketExt) -> R,
    {
        self.socket.lock_with(|socket| f(socket))
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
}

impl AnySocket for TcpSocket {
    type RawSocket = RawTcpSocket;

    fn new(socket: Box<Self::RawSocket>) -> Self {
        let socket_ext = RawTcpSocketExt {
            socket,
            in_background: false,
        };

        Self {
            socket: SpinLock::new(socket_ext),
            is_dead: AtomicBool::new(false),
        }
    }

    fn on_drop<E>(this: &Arc<BoundSocketInner<Self, E>>) {
        this.socket.lock_with(|socket| {
            socket.in_background = true;
            socket.close();

            // A TCP socket may not be appropriate for immediate removal. We leave the removal decision
            // to the polling logic.
            this.update_next_poll_at_ms(PollAt::Now);
            this.socket.update_dead(socket);
        });
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
        this.socket.lock_with(|socket| socket.close());

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
            next_poll_at_ms: AtomicU64::new(u64::MAX),
            has_new_events: AtomicBool::new(false),
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
        *self.0.observer.write_irq_disabled() = new_observer;

        self.0.on_iface_events();
    }

    /// Returns the observer.
    ///
    /// See also [`Self::set_observer`].
    pub fn observer(&self) -> Weak<dyn SocketEventObserver> {
        // We never hold the write lock in IRQ handlers, so we don't need to disable IRQs when we
        // get the read lock.
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

impl<E> BoundTcpSocket<E> {
    /// Connects to a remote endpoint.
    pub fn connect(
        &self,
        remote_endpoint: IpEndpoint,
    ) -> Result<(), smoltcp::socket::tcp::ConnectError> {
        let common = self.iface().common();
        common.lock_with(|iface| {
            self.0.socket.lock_with(|socket| {
                let result = socket.connect(iface.context(), remote_endpoint, self.0.port);
                self.0
                    .update_next_poll_at_ms(socket.poll_at(iface.context()));

                result
            })
        })
    }

    /// Listens at a specified endpoint.
    pub fn listen(
        &self,
        local_endpoint: IpEndpoint,
    ) -> Result<(), smoltcp::socket::tcp::ListenError> {
        self.0
            .socket
            .lock_with(|socket| socket.listen(local_endpoint))
    }

    pub fn send<F, R>(&self, f: F) -> Result<R, smoltcp::socket::tcp::SendError>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        self.0.socket.lock_with(|socket| {
            let result = socket.send(f);
            self.0.update_next_poll_at_ms(PollAt::Now);

            result
        })
    }

    pub fn recv<F, R>(&self, f: F) -> Result<R, smoltcp::socket::tcp::RecvError>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        self.0.socket.lock_with(|socket| {
            let result = socket.recv(f);
            self.0.update_next_poll_at_ms(PollAt::Now);

            result
        })
    }

    pub fn close(&self) {
        self.0.socket.lock_with(|socket| {
            socket.close();
            self.0.update_next_poll_at_ms(PollAt::Now);
        });
    }

    /// Calls `f` with an immutable reference to the associated [`RawTcpSocket`].
    //
    // NOTE: If a mutable reference is required, add a method above that correctly updates the next
    // polling time.
    pub fn raw_with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&RawTcpSocket) -> R,
    {
        self.0.socket.lock_with(|socket| f(socket))
    }
}

impl<E> BoundUdpSocket<E> {
    /// Binds to a specified endpoint.
    pub fn bind(&self, local_endpoint: IpEndpoint) -> Result<(), smoltcp::socket::udp::BindError> {
        self.0
            .socket
            .lock_with(|socket| socket.bind(local_endpoint))
    }

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

        self.0.socket.lock_with(|socket| {
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
        })
    }

    pub fn recv<F, R>(&self, f: F) -> Result<R, smoltcp::socket::udp::RecvError>
    where
        F: FnOnce(&[u8], UdpMetadata) -> R,
    {
        self.0.socket.lock_with(|socket| {
            let (data, meta) = socket.recv()?;
            let result = f(data, meta);
            self.0.update_next_poll_at_ms(PollAt::Now);

            Ok(result)
        })
    }

    /// Calls `f` with an immutable reference to the associated [`RawUdpSocket`].
    //
    // NOTE: If a mutable reference is required, add a method above that correctly updates the next
    // polling time.
    pub fn raw_with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&RawUdpSocket) -> R,
    {
        self.0.socket.lock_with(|socket| f(socket))
    }
}

impl<T, E> BoundSocketInner<T, E> {
    pub(crate) fn has_new_events(&self) -> bool {
        self.has_new_events.load(Ordering::Relaxed)
    }

    pub(crate) fn on_iface_events(&self) {
        self.has_new_events.store(false, Ordering::Relaxed);

        // We never hold the write lock in IRQ handlers, so we don't need to disable IRQs when we
        // get the read lock.
        let observer = Weak::upgrade(&*self.observer.read());

        if let Some(inner) = observer {
            inner.on_events();
        }
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
    fn update_next_poll_at_ms(&self, poll_at: PollAt) {
        self.has_new_events.store(true, Ordering::Relaxed);

        match poll_at {
            PollAt::Now => self.next_poll_at_ms.store(0, Ordering::Relaxed),
            PollAt::Time(instant) => self
                .next_poll_at_ms
                .store(instant.total_millis() as u64, Ordering::Relaxed),
            PollAt::Ingress => self.next_poll_at_ms.store(u64::MAX, Ordering::Relaxed),
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
        self.socket.lock_with(|socket| {
            if !socket.accepts(cx, ip_repr, tcp_repr) {
                return TcpProcessResult::NotProcessed;
            }

            let result = match socket.process(cx, ip_repr, tcp_repr) {
                None => TcpProcessResult::Processed,
                Some((ip_repr, tcp_repr)) => {
                    TcpProcessResult::ProcessedWithReply(ip_repr, tcp_repr)
                }
            };

            self.update_next_poll_at_ms(socket.poll_at(cx));
            self.socket.update_dead(socket);

            result
        })
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
        self.socket.lock_with(|socket| {
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
            }

            self.update_next_poll_at_ms(socket.poll_at(cx));
            self.socket.update_dead(socket);

            reply
        })
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
        self.socket.lock_with(|socket| {
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
            self.update_next_poll_at_ms(socket.poll_at(cx));

            true
        })
    }

    /// Tries to generate an outgoing packet and dispatches the generated packet.
    pub(crate) fn dispatch<D>(&self, cx: &mut Context, dispatch: D)
    where
        D: FnOnce(&mut Context, &IpRepr, &UdpRepr, &[u8]),
    {
        self.socket.lock_with(|socket| {
            socket
                .dispatch(cx, |cx, _meta, (ip_repr, udp_repr, udp_payload)| {
                    dispatch(cx, &ip_repr, &udp_repr, udp_payload);
                    Ok::<(), ()>(())
                })
                .unwrap();
            self.update_next_poll_at_ms(socket.poll_at(cx));
        });
    }
}
