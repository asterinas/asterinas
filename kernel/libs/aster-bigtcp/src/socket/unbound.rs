// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, vec};

use super::{option::RawTcpSetOption, NeedIfacePoll, RawTcpSocket, RawUdpSocket};

pub struct UnboundSocket<T> {
    socket: Box<T>,
}

pub type UnboundTcpSocket = UnboundSocket<RawTcpSocket>;
pub type UnboundUdpSocket = UnboundSocket<RawUdpSocket>;

impl UnboundTcpSocket {
    pub fn new() -> Self {
        let raw_tcp_socket = {
            let rx_buffer = smoltcp::socket::tcp::SocketBuffer::new(vec![0u8; TCP_RECV_BUF_LEN]);
            let tx_buffer = smoltcp::socket::tcp::SocketBuffer::new(vec![0u8; TCP_SEND_BUF_LEN]);
            RawTcpSocket::new(rx_buffer, tx_buffer)
        };
        Self {
            socket: Box::new(raw_tcp_socket),
        }
    }
}

impl Default for UnboundTcpSocket {
    fn default() -> Self {
        Self::new()
    }
}

impl RawTcpSetOption for UnboundTcpSocket {
    fn set_keep_alive(&mut self, interval: Option<smoltcp::time::Duration>) -> NeedIfacePoll {
        self.socket.set_keep_alive(interval);
        NeedIfacePoll::FALSE
    }

    fn set_nagle_enabled(&mut self, enabled: bool) {
        self.socket.set_nagle_enabled(enabled);
    }
}

impl UnboundUdpSocket {
    pub fn new() -> Self {
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
        }
    }
}

impl Default for UnboundUdpSocket {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> UnboundSocket<T> {
    pub(crate) fn into_raw(self) -> Box<T> {
        self.socket
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
