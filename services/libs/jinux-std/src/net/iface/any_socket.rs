use crate::{
    fs::utils::{IoEvents, Pollee, Poller},
    prelude::*,
};

use super::Iface;
use super::{IpAddress, IpEndpoint};

pub type RawTcpSocket = smoltcp::socket::tcp::Socket<'static>;
pub type RawUdpSocket = smoltcp::socket::udp::Socket<'static>;
pub type RawSocketHandle = smoltcp::iface::SocketHandle;

pub struct AnyUnboundSocket {
    socket_family: AnyRawSocket,
    pollee: Pollee,
}

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
        let pollee = Pollee::new(IoEvents::empty());
        AnyUnboundSocket {
            socket_family: AnyRawSocket::Tcp(raw_tcp_socket),
            pollee,
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
            pollee: Pollee::new(IoEvents::empty()),
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

    pub fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
    }

    pub(super) fn pollee(&self) -> Pollee {
        self.pollee.clone()
    }
}

pub struct AnyBoundSocket {
    iface: Arc<dyn Iface>,
    handle: smoltcp::iface::SocketHandle,
    port: u16,
    pollee: Pollee,
    socket_family: SocketFamily,
    weak_self: Weak<Self>,
}

impl AnyBoundSocket {
    pub(super) fn new(
        iface: Arc<dyn Iface>,
        handle: smoltcp::iface::SocketHandle,
        port: u16,
        pollee: Pollee,
        socket_family: SocketFamily,
    ) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            iface,
            handle,
            port,
            pollee,
            socket_family,
            weak_self: weak_self.clone(),
        })
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

    pub fn update_socket_state(&self) {
        let handle = &self.handle;
        let pollee = &self.pollee;
        let sockets = self.iface().sockets();
        match self.socket_family {
            SocketFamily::Tcp => {
                let socket = sockets.get::<RawTcpSocket>(*handle);
                update_tcp_socket_state(socket, pollee);
            }
            SocketFamily::Udp => {
                let udp_socket = sockets.get::<RawUdpSocket>(*handle);
                update_udp_socket_state(udp_socket, pollee);
            }
        }
    }

    pub fn iface(&self) -> &Arc<dyn Iface> {
        &self.iface
    }

    pub fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
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

fn update_tcp_socket_state(socket: &RawTcpSocket, pollee: &Pollee) {
    if socket.can_recv() {
        pollee.add_events(IoEvents::IN);
    } else {
        pollee.del_events(IoEvents::IN);
    }

    if socket.can_send() {
        pollee.add_events(IoEvents::OUT);
    } else {
        pollee.del_events(IoEvents::OUT);
    }

    if socket.may_recv() {
        pollee.del_events(IoEvents::RDHUP);
    } else {
        // The receice half was closed
        pollee.add_events(IoEvents::RDHUP);
    }

    if socket.is_open() {
        pollee.del_events(IoEvents::HUP);
    } else {
        // The socket is closed
        pollee.add_events(IoEvents::HUP);
    }
}

fn update_udp_socket_state(socket: &RawUdpSocket, pollee: &Pollee) {
    if socket.can_recv() {
        pollee.add_events(IoEvents::IN);
    } else {
        pollee.del_events(IoEvents::IN);
    }

    if socket.can_send() {
        pollee.add_events(IoEvents::OUT);
    } else {
        pollee.del_events(IoEvents::OUT);
    }
}

// For TCP
const RECV_BUF_LEN: usize = 65536;
const SEND_BUF_LEN: usize = 65536;

// For UDP
const UDP_METADATA_LEN: usize = 256;
const UDP_SEND_PAYLOAD_LEN: usize = 65536;
const UDP_RECEIVE_PAYLOAD_LEN: usize = 65536;
