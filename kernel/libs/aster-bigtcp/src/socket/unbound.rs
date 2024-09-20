// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Weak, vec};

use super::{event::SocketEventObserver, RawTcpSocket, RawUdpSocket};

pub struct UnboundSocket<T> {
    socket: Box<T>,
    observer: Weak<dyn SocketEventObserver>,
}

pub type UnboundTcpSocket = UnboundSocket<RawTcpSocket>;
pub type UnboundUdpSocket = UnboundSocket<RawUdpSocket>;

impl UnboundTcpSocket {
    pub fn new(observer: Weak<dyn SocketEventObserver>) -> Self {
        let raw_tcp_socket = {
            let rx_buffer = smoltcp::socket::tcp::SocketBuffer::new(vec![0u8; TCP_RECV_BUF_LEN]);
            let tx_buffer = smoltcp::socket::tcp::SocketBuffer::new(vec![0u8; TCP_SEND_BUF_LEN]);
            RawTcpSocket::new(rx_buffer, tx_buffer)
        };
        Self {
            socket: Box::new(raw_tcp_socket),
            observer,
        }
    }
}

impl UnboundUdpSocket {
    pub fn new(observer: Weak<dyn SocketEventObserver>) -> Self {
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
        Self {
            socket: Box::new(raw_udp_socket),
            observer,
        }
    }
}

impl<T> UnboundSocket<T> {
    pub(crate) fn into_raw(self) -> (Box<T>, Weak<dyn SocketEventObserver>) {
        (self.socket, self.observer)
    }
}

// For TCP
pub const TCP_RECV_BUF_LEN: usize = 65536;
pub const TCP_SEND_BUF_LEN: usize = 65536;

// For UDP
pub const UDP_SEND_PAYLOAD_LEN: usize = 65536;
pub const UDP_RECV_PAYLOAD_LEN: usize = 65536;
const UDP_METADATA_LEN: usize = 256;
