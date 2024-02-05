// SPDX-License-Identifier: MPL-2.0

use crate::events::Observer;
use crate::prelude::*;

use super::Iface;
use super::{IpAddress, IpEndpoint};

pub type RawTcpSocket = smoltcp::socket::tcp::Socket<'static>;
pub type RawUdpSocket = smoltcp::socket::udp::Socket<'static>;
pub type RawSocketHandle = smoltcp::iface::SocketHandle;

pub struct AnyUnboundSocket {
    socket_family: AnyRawSocket,
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
    pub fn new_tcp() -> Self {
        let raw_tcp_socket = {
            let rx_buffer = smoltcp::socket::tcp::SocketBuffer::new(vec![0u8; RECV_BUF_LEN]);
            let tx_buffer = smoltcp::socket::tcp::SocketBuffer::new(vec![0u8; SEND_BUF_LEN]);
            RawTcpSocket::new(rx_buffer, tx_buffer)
        };
        AnyUnboundSocket {
            socket_family: AnyRawSocket::Tcp(raw_tcp_socket),
        }
    }

    pub fn new_udp() -> Self {
        let raw_udp_socket = {
            let metadata = smoltcp::socket::udp::PacketMetadata::EMPTY;
            let rx_buffer = smoltcp::socket::udp::PacketBuffer::new(
                vec![metadata; UDP_METADATA_LEN],
                vec![0u8; UDP_RECEIVE_PAYLOAD_LEN],
            );
            let tx_buffer = smoltcp::socket::udp::PacketBuffer::new(
                vec![metadata; UDP_METADATA_LEN],
                vec![0u8; UDP_RECEIVE_PAYLOAD_LEN],
            );
            RawUdpSocket::new(rx_buffer, tx_buffer)
        };
        AnyUnboundSocket {
            socket_family: AnyRawSocket::Udp(raw_udp_socket),
        }
    }

    pub(super) fn raw_socket_family(self) -> AnyRawSocket {
        self.socket_family
    }

    pub(super) fn socket_family(&self) -> SocketFamily {
        match &self.socket_family {
            AnyRawSocket::Tcp(_) => SocketFamily::Tcp,
            AnyRawSocket::Udp(_) => SocketFamily::Udp,
        }
    }
}

pub struct AnyBoundSocket {
    iface: Arc<dyn Iface>,
    handle: smoltcp::iface::SocketHandle,
    port: u16,
    socket_family: SocketFamily,
    observer: RwLock<Weak<dyn Observer<()>>>,
    weak_self: Weak<Self>,
}

impl AnyBoundSocket {
    pub(super) fn new(
        iface: Arc<dyn Iface>,
        handle: smoltcp::iface::SocketHandle,
        port: u16,
        socket_family: SocketFamily,
    ) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            iface,
            handle,
            port,
            socket_family,
            observer: RwLock::new(Weak::<()>::new()),
            weak_self: weak_self.clone(),
        })
    }

    pub(super) fn on_iface_events(&self) {
        if let Some(observer) = Weak::upgrade(&*self.observer.read()) {
            observer.on_events(&())
        }
    }

    /// Set the observer whose `on_events` will be called when certain iface events happen. After
    /// setting, the new observer will fire once immediately to avoid missing any events.
    ///
    /// If there is an existing observer, due to race conditions, this function does not guarentee
    /// that the old observer will never be called after the setting. Users should be aware of this
    /// and proactively handle the race conditions if necessary.
    pub fn set_observer(&self, handler: Weak<dyn Observer<()>>) {
        *self.observer.write() = handler;

        self.on_iface_events();
    }

    pub fn local_endpoint(&self) -> Option<IpEndpoint> {
        let ip_addr = {
            let ipv4_addr = self.iface.ipv4_addr()?;
            IpAddress::Ipv4(ipv4_addr)
        };
        Some(IpEndpoint::new(ip_addr, self.port))
    }

    pub fn raw_with<T: smoltcp::socket::AnySocket<'static>, R, F: FnMut(&mut T) -> R>(
        &self,
        mut f: F,
    ) -> R {
        let mut sockets = self.iface.sockets();
        let socket = sockets.get_mut::<T>(self.handle);
        f(socket)
    }

    /// Try to connect to a remote endpoint. Tcp socket only.
    pub fn do_connect(&self, remote_endpoint: IpEndpoint) -> Result<()> {
        let mut sockets = self.iface.sockets();
        let socket = sockets.get_mut::<RawTcpSocket>(self.handle);
        let port = self.port;
        let mut iface_inner = self.iface.iface_inner();
        let cx = iface_inner.context();
        socket
            .connect(cx, remote_endpoint, port)
            .map_err(|_| Error::with_message(Errno::ENOBUFS, "send connection request failed"))?;
        Ok(())
    }

    pub fn iface(&self) -> &Arc<dyn Iface> {
        &self.iface
    }

    pub(super) fn weak_ref(&self) -> Weak<Self> {
        self.weak_self.clone()
    }

    fn close(&self) {
        match self.socket_family {
            SocketFamily::Tcp => self.raw_with(|socket: &mut RawTcpSocket| socket.close()),
            SocketFamily::Udp => self.raw_with(|socket: &mut RawUdpSocket| socket.close()),
        }
    }
}

impl Drop for AnyBoundSocket {
    fn drop(&mut self) {
        self.close();
        self.iface.poll();
        self.iface.common().remove_socket(self.handle);
        self.iface.common().release_port(self.port);
        self.iface.common().remove_bound_socket(self.weak_ref());
    }
}

// For TCP
pub const RECV_BUF_LEN: usize = 65536;
pub const SEND_BUF_LEN: usize = 65536;

// For UDP
const UDP_METADATA_LEN: usize = 256;
const UDP_SEND_PAYLOAD_LEN: usize = 65536;
const UDP_RECEIVE_PAYLOAD_LEN: usize = 65536;
