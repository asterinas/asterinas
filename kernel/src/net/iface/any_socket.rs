// SPDX-License-Identifier: MPL-2.0

use super::Iface;
use crate::{
    events::Observer,
    net::socket::ip::{IpAddress, IpEndpoint},
    prelude::*,
};

pub type RawTcpSocket = smoltcp::socket::tcp::Socket<'static>;
pub type RawUdpSocket = smoltcp::socket::udp::Socket<'static>;

pub struct AnyUnboundSocket {
    socket_family: AnyRawSocket,
    observer: Weak<dyn Observer<()>>,
}

#[allow(clippy::large_enum_variant)]
pub(super) enum AnyRawSocket {
    Tcp(RawTcpSocket),
    Udp(RawUdpSocket),
}

pub(super) enum SocketFamily {
    Tcp,
    Udp,
}

impl AnyUnboundSocket {
    pub fn new_tcp(observer: Weak<dyn Observer<()>>) -> Self {
        let raw_tcp_socket = {
            let rx_buffer = smoltcp::socket::tcp::SocketBuffer::new(vec![0u8; TCP_RECV_BUF_LEN]);
            let tx_buffer = smoltcp::socket::tcp::SocketBuffer::new(vec![0u8; TCP_SEND_BUF_LEN]);
            RawTcpSocket::new(rx_buffer, tx_buffer)
        };
        AnyUnboundSocket {
            socket_family: AnyRawSocket::Tcp(raw_tcp_socket),
            observer,
        }
    }

    pub fn new_udp(observer: Weak<dyn Observer<()>>) -> Self {
        let raw_udp_socket = {
            let metadata = smoltcp::socket::udp::PacketMetadata::EMPTY;
            let rx_buffer = smoltcp::socket::udp::PacketBuffer::new(
                vec![metadata; UDP_METADATA_LEN],
                vec![0u8; UDP_RECV_PAYLOAD_LEN],
            );
            let tx_buffer = smoltcp::socket::udp::PacketBuffer::new(
                vec![metadata; UDP_METADATA_LEN],
                vec![0u8; UDP_SEND_PAYLOAD_LEN],
            );
            RawUdpSocket::new(rx_buffer, tx_buffer)
        };
        AnyUnboundSocket {
            socket_family: AnyRawSocket::Udp(raw_udp_socket),
            observer,
        }
    }

    pub(super) fn into_raw(self) -> (AnyRawSocket, Weak<dyn Observer<()>>) {
        (self.socket_family, self.observer)
    }
}

pub struct AnyBoundSocket(Arc<AnyBoundSocketInner>);

impl AnyBoundSocket {
    pub(super) fn new(
        iface: Arc<dyn Iface>,
        handle: smoltcp::iface::SocketHandle,
        port: u16,
        socket_family: SocketFamily,
        observer: Weak<dyn Observer<()>>,
    ) -> Self {
        Self(Arc::new(AnyBoundSocketInner {
            iface,
            handle,
            port,
            socket_family,
            observer: RwLock::new(observer),
        }))
    }

    pub(super) fn inner(&self) -> &Arc<AnyBoundSocketInner> {
        &self.0
    }

    /// Set the observer whose `on_events` will be called when certain iface events happen. After
    /// setting, the new observer will fire once immediately to avoid missing any events.
    ///
    /// If there is an existing observer, due to race conditions, this function does not guarentee
    /// that the old observer will never be called after the setting. Users should be aware of this
    /// and proactively handle the race conditions if necessary.
    pub fn set_observer(&self, handler: Weak<dyn Observer<()>>) {
        *self.0.observer.write() = handler;

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

    /// Try to connect to a remote endpoint. Tcp socket only.
    pub fn do_connect(&self, remote_endpoint: IpEndpoint) -> Result<()> {
        let mut sockets = self.0.iface.sockets();
        let socket = sockets.get_mut::<RawTcpSocket>(self.0.handle);
        let port = self.0.port;
        let mut iface_inner = self.0.iface.iface_inner();
        let cx = iface_inner.context();
        socket
            .connect(cx, remote_endpoint, port)
            .map_err(|_| Error::with_message(Errno::ENOBUFS, "send connection request failed"))?;
        Ok(())
    }

    pub fn iface(&self) -> &Arc<dyn Iface> {
        &self.0.iface
    }
}

impl Drop for AnyBoundSocket {
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

pub(super) struct AnyBoundSocketInner {
    iface: Arc<dyn Iface>,
    handle: smoltcp::iface::SocketHandle,
    port: u16,
    socket_family: SocketFamily,
    observer: RwLock<Weak<dyn Observer<()>>>,
}

impl AnyBoundSocketInner {
    pub(super) fn on_iface_events(&self) {
        if let Some(observer) = Weak::upgrade(&*self.observer.read()) {
            observer.on_events(&())
        }
    }

    pub(super) fn is_closed(&self) -> bool {
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
        let mut sockets = self.iface.sockets();
        let socket = sockets.get_mut::<T>(self.handle);
        f(socket)
    }
}

impl Drop for AnyBoundSocketInner {
    fn drop(&mut self) {
        let iface_common = self.iface.common();
        iface_common.remove_socket(self.handle);
        iface_common.release_port(self.port);
    }
}

// For TCP
pub const TCP_RECV_BUF_LEN: usize = 65536;
pub const TCP_SEND_BUF_LEN: usize = 65536;

// For UDP
pub const UDP_SEND_PAYLOAD_LEN: usize = 65536;
pub const UDP_RECV_PAYLOAD_LEN: usize = 65536;
const UDP_METADATA_LEN: usize = 256;
