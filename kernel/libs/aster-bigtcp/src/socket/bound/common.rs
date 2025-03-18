// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};
use core::sync::atomic::{AtomicU8, Ordering};

use smoltcp::wire::IpEndpoint;
use spin::once::Once;
use takeable::Takeable;

use crate::{
    define_boolean_value,
    ext::Ext,
    iface::{BoundPort, Iface},
    socket::event::{SocketEventObserver, SocketEvents},
};

pub struct Socket<T: Inner<E>, E: Ext>(pub(super) Takeable<Arc<SocketBg<T, E>>>);

/// [`TcpConnectionInner`], [`TcpListenerInner`], or [`UdpSocketInner`].
///
/// [`TcpConnectionInner`]: super::tcp_conn::TcpConnectionInner
/// [`TcpListenerInner`]: super::tcp_listen::TcpListenerInner
/// [`UdpSocketInner`]: super::udp::UdpSocketInner
pub trait Inner<E: Ext> {
    type Observer: SocketEventObserver;

    /// Called by [`Socket::drop`].
    fn on_drop(this: &Arc<SocketBg<Self, E>>)
    where
        E: Ext,
        Self: Sized;
}

/// Common states shared by [`TcpConnectionBg`], [`TcpListenerBg`], and [`UdpSocketBg`].
///
/// In the type name, `Bg` means "background". Its meaning is described below:
/// - A foreground socket (e.g., [`TcpConnection`]) handles system calls from the user program.
/// - A background socket (e.g., [`TcpConnectionBg`]) handles packets from the network.
///
/// [`TcpConnectionBg`]: super::tcp_conn::TcpConnectionBg
/// [`TcpListenerBg`]: super::tcp_listen::TcpListenerBg
/// [`UdpSocketBg`]: super::udp::UdpSocketBg
/// [`TcpConnection`]: super::tcp_conn::TcpConnection
pub struct SocketBg<T: Inner<E>, E: Ext> {
    pub(super) bound: BoundPort<E>,
    pub(super) inner: T,
    observer: Once<T::Observer>,
    events: AtomicU8,
}

impl<T: Inner<E>, E: Ext> Drop for Socket<T, E> {
    fn drop(&mut self) {
        if self.0.is_usable() {
            T::on_drop(&self.0);
        }
    }
}

impl<T: Inner<E>, E: Ext> Socket<T, E> {
    pub(super) fn new(bound: BoundPort<E>, inner: T) -> Self {
        Self(Takeable::new(Arc::new(SocketBg {
            bound,
            inner,
            observer: Once::new(),
            events: AtomicU8::new(0),
        })))
    }

    pub(super) fn new_cyclic<F>(bound: BoundPort<E>, inner_fn: F) -> Self
    where
        F: FnOnce(&Weak<SocketBg<T, E>>) -> T,
    {
        Self(Takeable::new(Arc::new_cyclic(|weak| SocketBg {
            bound,
            inner: inner_fn(weak),
            observer: Once::new(),
            events: AtomicU8::new(0),
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

define_boolean_value!(
    /// Whether the iface needs to be polled
    NeedIfacePoll
);

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
        // There is no need to clear the events because the socket is dead.
        let events = self.events.load(Ordering::Relaxed);

        let observer = self.observer.get().cloned();
        drop(self);

        // Notify dead events after the `Arc` is dropped to ensure the observer sees this event
        // with the expected reference count. See `TcpConnection::connect_state` for an example.
        if let Some(ref observer) = observer {
            observer.on_events(SocketEvents::from_bits_truncate(events));
        }
    }

    pub(super) fn add_events(&self, new_events: SocketEvents) {
        // This method can only be called to add network events, so we assume we are holding the
        // poll lock and no race conditions can occur.
        let events = self.events.load(Ordering::Relaxed);
        self.events
            .store(events | new_events.bits(), Ordering::Relaxed);
    }
}

impl<T: Inner<E>, E: Ext> SocketBg<T, E> {
    /// Returns whether an incoming packet _may_ be processed by the socket.
    ///
    /// The check is intended to be lock-free and fast, but may have false positives.
    pub(crate) fn can_process(&self, dst_port: u16) -> bool {
        self.bound.port() == dst_port
    }
}
