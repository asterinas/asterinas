// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use aster_bigtcp::socket::{
    NeedIfacePoll, TCP_RECV_BUF_LEN, TCP_SEND_BUF_LEN, UDP_RECV_PAYLOAD_LEN, UDP_SEND_PAYLOAD_LEN,
};

use crate::{
    match_sock_option_mut, match_sock_option_ref,
    net::socket::options::{
        KeepAlive, Linger, RecvBuf, ReuseAddr, ReusePort, SendBuf, SocketOption,
    },
    prelude::*,
};

#[derive(Debug, Clone, CopyGetters, Setters)]
#[get_copy = "pub"]
#[set = "pub"]
pub struct SocketOptionSet {
    reuse_addr: bool,
    reuse_port: bool,
    send_buf: u32,
    recv_buf: u32,
    linger: LingerOption,
    keep_alive: bool,
}

impl SocketOptionSet {
    /// Return the default socket level options for tcp socket.
    pub fn new_tcp() -> Self {
        Self {
            reuse_addr: false,
            reuse_port: false,
            send_buf: TCP_SEND_BUF_LEN as u32,
            recv_buf: TCP_RECV_BUF_LEN as u32,
            linger: LingerOption::default(),
            keep_alive: false,
        }
    }

    /// Return the default socket level options for udp socket.
    pub fn new_udp() -> Self {
        Self {
            reuse_addr: false,
            reuse_port: false,
            send_buf: UDP_SEND_PAYLOAD_LEN as u32,
            recv_buf: UDP_RECV_PAYLOAD_LEN as u32,
            linger: LingerOption::default(),
            keep_alive: false,
        }
    }

    /// Gets socket-level options.
    ///
    /// Note that the socket error has to be handled separately, because it is automatically
    /// cleared after reading. This method does not handle it. Instead,
    /// [`Self::get_and_clear_socket_errors`] should be used.
    pub fn get_option(&self, option: &mut dyn SocketOption) -> Result<()> {
        match_sock_option_mut!(option, {
            socket_reuse_addr: ReuseAddr => {
                let reuse_addr = self.reuse_addr();
                socket_reuse_addr.set(reuse_addr);
            },
            socket_send_buf: SendBuf => {
                let send_buf = self.send_buf();
                socket_send_buf.set(send_buf);
            },
            socket_recv_buf: RecvBuf => {
                let recv_buf = self.recv_buf();
                socket_recv_buf.set(recv_buf);
            },
            socket_reuse_port: ReusePort => {
                let reuse_port = self.reuse_port();
                socket_reuse_port.set(reuse_port);
            },
            socket_linger: Linger => {
                let linger = self.linger();
                socket_linger.set(linger);
            },
            socket_keepalive: KeepAlive => {
                let keep_alive = self.keep_alive();
                socket_keepalive.set(keep_alive);
            },
            _ => return_errno_with_message!(Errno::ENOPROTOOPT, "the socket option to get is unknown")
        });
        Ok(())
    }

    /// Sets socket-level options.
    pub fn set_option(
        &mut self,
        option: &dyn SocketOption,
        socket: &dyn SetSocketLevelOption,
    ) -> Result<NeedIfacePoll> {
        match_sock_option_ref!(option, {
            socket_recv_buf: RecvBuf => {
                let recv_buf = socket_recv_buf.get().unwrap();
                if *recv_buf <= MIN_RECVBUF {
                    self.set_recv_buf(MIN_RECVBUF);
                } else {
                    self.set_recv_buf(*recv_buf);
                }
            },
            socket_send_buf: SendBuf => {
                let send_buf = socket_send_buf.get().unwrap();
                if *send_buf <= MIN_SENDBUF {
                    self.set_send_buf(MIN_SENDBUF);
                } else {
                    self.set_send_buf(*send_buf);
                }
            },
            socket_reuse_addr: ReuseAddr => {
                let reuse_addr = socket_reuse_addr.get().unwrap();
                self.set_reuse_addr(*reuse_addr);
            },
            socket_reuse_port: ReusePort => {
                let reuse_port = socket_reuse_port.get().unwrap();
                self.set_reuse_port(*reuse_port);
            },
            socket_linger: Linger => {
                let linger = socket_linger.get().unwrap();
                self.set_linger(*linger);
            },
            socket_keepalive: KeepAlive => {
                let keep_alive = socket_keepalive.get().unwrap();
                self.set_keep_alive(*keep_alive);
                return Ok(socket.set_keep_alive(*keep_alive));
            },
            _ => return_errno_with_message!(Errno::ENOPROTOOPT, "the socket option to be set is unknown")
        });

        Ok(NeedIfacePoll::FALSE)
    }
}

pub const MIN_SENDBUF: u32 = 2304;
pub const MIN_RECVBUF: u32 = 2304;

#[derive(Debug, Default, Clone, Copy)]
pub struct LingerOption {
    is_on: bool,
    timeout: Duration,
}

impl LingerOption {
    pub fn new(is_on: bool, timeout: Duration) -> Self {
        Self { is_on, timeout }
    }

    pub fn is_on(&self) -> bool {
        self.is_on
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

/// A trait used for setting socket level options on actual sockets.
pub(in crate::net) trait SetSocketLevelOption {
    /// Sets whether keepalive messages are enabled.
    fn set_keep_alive(&self, _keep_alive: bool) -> NeedIfacePoll {
        NeedIfacePoll::FALSE
    }
}
