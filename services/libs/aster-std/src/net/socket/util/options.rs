use core::time::Duration;

use crate::net::iface::{RECV_BUF_LEN, SEND_BUF_LEN};
use crate::prelude::*;

#[derive(Debug, Clone, CopyGetters, Setters)]
#[get_copy = "pub"]
#[set = "pub"]
pub struct SocketOptionSet {
    sock_errors: Option<Error>,
    reuse_addr: bool,
    reuse_port: bool,
    send_buf: u32,
    recv_buf: u32,
    linger: LingerOption,
}

impl SocketOptionSet {
    /// Return the default socket level options for tcp socket.
    pub fn new_tcp() -> Self {
        Self {
            sock_errors: None,
            reuse_addr: false,
            reuse_port: false,
            send_buf: SEND_BUF_LEN as u32,
            recv_buf: RECV_BUF_LEN as u32,
            linger: LingerOption::default(),
        }
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
