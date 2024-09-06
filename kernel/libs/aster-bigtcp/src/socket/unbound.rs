// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Weak, vec};

use super::{event::SocketEventObserver, RawTcpSocket, RawUdpSocket};

pub struct AnyUnboundSocket {
    socket_family: AnyRawSocket,
    observer: Weak<dyn SocketEventObserver>,
}

#[allow(clippy::large_enum_variant)]
pub(crate) enum AnyRawSocket {
    Tcp(RawTcpSocket),
    Udp(RawUdpSocket),
}

impl AnyUnboundSocket {
    pub fn new_tcp(observer: Weak<dyn SocketEventObserver>) -> Self {
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

    pub fn new_udp(observer: Weak<dyn SocketEventObserver>) -> Self {
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

    pub(crate) fn into_raw(self) -> (AnyRawSocket, Weak<dyn SocketEventObserver>) {
        (self.socket_family, self.observer)
    }
}

// For TCP
pub const TCP_RECV_BUF_LEN: usize = 65536;
pub const TCP_SEND_BUF_LEN: usize = 65536;

// For UDP
pub const UDP_SEND_PAYLOAD_LEN: usize = 65536;
pub const UDP_RECV_PAYLOAD_LEN: usize = 65536;
const UDP_METADATA_LEN: usize = 256;
