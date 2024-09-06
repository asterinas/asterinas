// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};

use ostd::sync::RwLock;
use smoltcp::{
    socket::tcp::ConnectError,
    wire::{IpAddress, IpEndpoint},
};

use super::{event::SocketEventObserver, RawTcpSocket, RawUdpSocket};
use crate::iface::Iface;

pub(crate) enum SocketFamily {
    Tcp,
    Udp,
}

pub struct AnyBoundSocket<E>(Arc<AnyBoundSocketInner<E>>);

impl<E> AnyBoundSocket<E> {
    pub(crate) fn new(
        iface: Arc<dyn Iface<E>>,
        handle: smoltcp::iface::SocketHandle,
        port: u16,
        socket_family: SocketFamily,
        observer: Weak<dyn SocketEventObserver>,
    ) -> Self {
        Self(Arc::new(AnyBoundSocketInner {
            iface,
            handle,
            port,
            socket_family,
            observer: RwLock::new(observer),
        }))
    }

    pub(crate) fn inner(&self) -> &Arc<AnyBoundSocketInner<E>> {
        &self.0
    }

    /// Sets the observer whose `on_events` will be called when certain iface events happen. After
    /// setting, the new observer will fire once immediately to avoid missing any events.
    ///
    /// If there is an existing observer, due to race conditions, this function does not guarantee
    /// that the old observer will never be called after the setting. Users should be aware of this
    /// and proactively handle the race conditions if necessary.
    pub fn set_observer(&self, new_observer: Weak<dyn SocketEventObserver>) {
        *self.0.observer.write() = new_observer;

        self.0.on_iface_events();
    }

    pub fn local_endpoint(&self) -> Option<IpEndpoint> {
        let ip_addr = {
            let ipv4_addr = self.0.iface.ipv4_addr()?;
            IpAddress::Ipv4(ipv4_addr)
        };
        Some(IpEndpoint::new(ip_addr, self.0.port))
    }

    pub fn raw_with<T: smoltcp::socket::AnySocket<'static>, R, F: FnMut(&mut T) -> R>(
        &self,
        f: F,
    ) -> R {
        self.0.raw_with(f)
    }

    /// Connects to a remote endpoint.
    ///
    /// # Panics
    ///
    /// This method will panic if the socket is not a TCP socket.
    pub fn do_connect(&self, remote_endpoint: IpEndpoint) -> Result<(), ConnectError> {
        let common = self.iface().common();

        let mut sockets = common.sockets();
        let socket = sockets.get_mut::<RawTcpSocket>(self.0.handle);

        let mut iface = common.interface();
        let cx = iface.context();

        socket.connect(cx, remote_endpoint, self.0.port)
    }

    pub fn iface(&self) -> &Arc<dyn Iface<E>> {
        &self.0.iface
    }
}

impl<E> Drop for AnyBoundSocket<E> {
    fn drop(&mut self) {
        if self.0.start_closing() {
            self.0.iface.common().remove_bound_socket_now(&self.0);
        } else {
            self.0
                .iface
                .common()
                .remove_bound_socket_when_closed(&self.0);
        }
    }
}

pub(crate) struct AnyBoundSocketInner<E> {
    iface: Arc<dyn Iface<E>>,
    handle: smoltcp::iface::SocketHandle,
    port: u16,
    socket_family: SocketFamily,
    observer: RwLock<Weak<dyn SocketEventObserver>>,
}

impl<E> AnyBoundSocketInner<E> {
    pub(crate) fn on_iface_events(&self) {
        if let Some(observer) = Weak::upgrade(&*self.observer.read()) {
            observer.on_events();
        }
    }

    pub(crate) fn is_closed(&self) -> bool {
        match self.socket_family {
            SocketFamily::Tcp => self.raw_with(|socket: &mut RawTcpSocket| {
                socket.state() == smoltcp::socket::tcp::State::Closed
            }),
            SocketFamily::Udp => true,
        }
    }

    /// Starts closing the socket and returns whether the socket is closed.
    ///
    /// For sockets that can be closed immediately, such as UDP sockets and TCP listening sockets,
    /// this method will always return `true`.
    ///
    /// For other sockets, such as TCP connected sockets, they cannot be closed immediately because
    /// we at least need to send the FIN packet and wait for the remote end to send an ACK packet.
    /// In this case, this method will return `false` and [`Self::is_closed`] can be used to
    /// determine if the closing process is complete.
    fn start_closing(&self) -> bool {
        match self.socket_family {
            SocketFamily::Tcp => self.raw_with(|socket: &mut RawTcpSocket| {
                socket.close();
                socket.state() == smoltcp::socket::tcp::State::Closed
            }),
            SocketFamily::Udp => {
                self.raw_with(|socket: &mut RawUdpSocket| socket.close());
                true
            }
        }
    }

    pub fn raw_with<T: smoltcp::socket::AnySocket<'static>, R, F: FnMut(&mut T) -> R>(
        &self,
        mut f: F,
    ) -> R {
        let mut sockets = self.iface.common().sockets();
        let socket = sockets.get_mut::<T>(self.handle);
        f(socket)
    }
}

impl<E> Drop for AnyBoundSocketInner<E> {
    fn drop(&mut self) {
        let iface_common = self.iface.common();
        iface_common.remove_socket(self.handle);
        iface_common.release_port(self.port);
    }
}
