// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc};
use core::sync::atomic::{AtomicBool, Ordering};

use aster_softirq::BottomHalfDisabled;
use ostd::sync::SpinLock;
use smoltcp::{
    iface::Context,
    socket::udp::UdpMetadata,
    wire::{IpRepr, UdpRepr},
};

use super::common::{Inner, Socket, SocketBg};
use crate::{
    errors::udp::SendError,
    ext::Ext,
    iface::BoundPort,
    socket::{event::SocketEvents, unbound::new_udp_socket, RawUdpSocket},
};

pub type UdpSocket<E> = Socket<UdpSocketInner, E>;

/// States needed by [`UdpSocketBg`].
pub struct UdpSocketInner {
    socket: SpinLock<Box<RawUdpSocket>, BottomHalfDisabled>,
    need_dispatch: AtomicBool,
}

impl<E: Ext> Inner<E> for UdpSocketInner {
    type Observer = E::UdpEventObserver;

    fn on_drop(this: &Arc<SocketBg<Self, E>>) {
        this.inner.socket.lock().close();

        // A UDP socket can be removed immediately.
        this.bound.iface().common().remove_udp_socket(this);
    }
}

pub(crate) type UdpSocketBg<E> = SocketBg<UdpSocketInner, E>;

impl<E: Ext> UdpSocketBg<E> {
    /// Tries to process an incoming packet and returns whether the packet is processed.
    pub(crate) fn process(
        &self,
        cx: &mut Context,
        ip_repr: &IpRepr,
        udp_repr: &UdpRepr,
        udp_payload: &[u8],
    ) -> bool {
        let mut socket = self.inner.socket.lock();

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

        self.notify_events(SocketEvents::CAN_RECV);

        true
    }

    /// Tries to generate an outgoing packet and dispatches the generated packet.
    pub(crate) fn dispatch<D>(&self, cx: &mut Context, dispatch: D)
    where
        D: FnOnce(&mut Context, &IpRepr, &UdpRepr, &[u8]),
    {
        let mut socket = self.inner.socket.lock();

        socket
            .dispatch(cx, |cx, _meta, (ip_repr, udp_repr, udp_payload)| {
                dispatch(cx, &ip_repr, &udp_repr, udp_payload);
                Ok::<(), ()>(())
            })
            .unwrap();

        // For UDP, dequeuing a packet means that we can queue more packets.
        self.notify_events(SocketEvents::CAN_SEND);

        self.inner
            .need_dispatch
            .store(socket.send_queue() > 0, Ordering::Relaxed);
    }

    /// Returns whether the socket _may_ generate an outgoing packet.
    ///
    /// The check is intended to be lock-free and fast, but may have false positives.
    pub(crate) fn need_dispatch(&self) -> bool {
        self.inner.need_dispatch.load(Ordering::Relaxed)
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

        let inner = UdpSocketInner {
            socket: SpinLock::new(socket),
            need_dispatch: AtomicBool::new(false),
        };

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
        let mut socket = self.0.inner.socket.lock();

        if size > socket.packet_send_capacity() {
            return Err(SendError::TooLarge);
        }

        let buffer = match socket.send(size, meta) {
            Ok(data) => data,
            Err(err) => return Err(err.into()),
        };
        let result = f(buffer);

        self.0
            .inner
            .need_dispatch
            .store(socket.send_queue() > 0, Ordering::Relaxed);

        Ok(result)
    }

    /// Receives some data.
    ///
    /// Polling the iface is _not_ required after this method succeeds.
    pub fn recv<F, R>(&self, f: F) -> Result<R, smoltcp::socket::udp::RecvError>
    where
        F: FnOnce(&[u8], UdpMetadata) -> R,
    {
        let mut socket = self.0.inner.socket.lock();

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
        let socket = self.0.inner.socket.lock();
        f(&socket)
    }
}
