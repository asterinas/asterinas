use core::sync::atomic::{AtomicBool, Ordering};

use crate::fs::utils::{IoEvents, Pollee, Poller};
use crate::net::socket::unix::addr::UnixSocketAddr;
use crate::prelude::*;

pub struct Init {
    is_nonblocking: AtomicBool,
    bind_addr: Option<UnixSocketAddr>,
    pollee: Pollee,
}

impl Init {
    pub fn new(is_nonblocking: bool) -> Self {
        Self {
            is_nonblocking: AtomicBool::new(is_nonblocking),
            bind_addr: None,
            pollee: Pollee::new(IoEvents::empty()),
        }
    }

    pub fn bind(&mut self, mut addr: UnixSocketAddr) -> Result<()> {
        if self.bind_addr.is_some() {
            return_errno_with_message!(Errno::EINVAL, "the socket is already bound");
        }
        addr.create_file_and_bind()?;
        self.bind_addr = Some(addr);
        Ok(())
    }

    pub fn is_bound(&self) -> bool {
        self.bind_addr.is_none()
    }

    pub fn bound_addr(&self) -> Option<&UnixSocketAddr> {
        self.bind_addr.as_ref()
    }

    pub fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Acquire)
    }

    pub fn set_nonblocking(&self, is_nonblocking: bool) {
        self.is_nonblocking.store(is_nonblocking, Ordering::Release);
    }

    pub fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
    }
}
