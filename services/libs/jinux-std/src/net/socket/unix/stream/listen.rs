use core::sync::atomic::{AtomicBool, Ordering};

use crate::net::socket::unix::addr::UnixSocketAddr;

pub struct Listen {
    addr: UnixSocketAddr,
    is_nonblocking: AtomicBool,
}

impl Listen {
    pub fn new(addr: UnixSocketAddr, nonblocking: bool) -> Self {
        Self {
            addr,
            is_nonblocking: AtomicBool::new(nonblocking),
        }
    }

    pub fn addr(&self) -> &UnixSocketAddr {
        &self.addr
    }

    pub fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Acquire)
    }

    pub fn set_nonblocking(&self, is_nonblocking: bool) {
        self.is_nonblocking.store(is_nonblocking, Ordering::Release);
    }
}
