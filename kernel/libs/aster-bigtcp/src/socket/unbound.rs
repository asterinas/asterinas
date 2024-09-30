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

// TCP socket buffer sizes:
//
// According to
// <https://github.com/torvalds/linux/blob/9852d85ec9d492ebef56dc5f229416c925758edc/include/net/sock.h#L2798-L2806>
// and
// <https://github.com/torvalds/linux/blob/9852d85ec9d492ebef56dc5f229416c925758edc/net/core/sock.c#L286-L287>,
// it seems that the socket buffer should be 256 packets * 256 bytes/packet = 65536 bytes by
// default. However, the loopback MTU is also 65536 bytes, and having the same size for the socket
// buffer and the MTU will cause the implementation of Nagle's algorithm in smoltcp to behave
// abnormally (see <https://github.com/asterinas/asterinas/pull/1396>). So the socket buffer size
// is increased from 64K to 128K.
//
// TODO: Consider allowing user programs to set the socket buffer length via `setsockopt` system calls.
pub const TCP_RECV_BUF_LEN: usize = 65536 * 2;
pub const TCP_SEND_BUF_LEN: usize = 65536 * 2;

// UDP socket buffer sizes:
pub const UDP_SEND_PAYLOAD_LEN: usize = 65536;
pub const UDP_RECV_PAYLOAD_LEN: usize = 65536;
const UDP_METADATA_LEN: usize = 256;
