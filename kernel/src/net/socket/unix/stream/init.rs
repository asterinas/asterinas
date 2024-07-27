// SPDX-License-Identifier: MPL-2.0

use crate::{
    events::{IoEvents, Observer},
    net::socket::unix::addr::{UnixSocketAddr, UnixSocketAddrBound},
    prelude::*,
    process::signal::{Pollee, Poller},
};

pub(super) struct Init {
    addr: Option<UnixSocketAddrBound>,
    pollee: Pollee,
}

impl Init {
    pub(super) fn new() -> Self {
        Self {
            addr: None,
            pollee: Pollee::new(IoEvents::empty()),
        }
    }

    pub(super) fn bind(&mut self, addr_to_bind: UnixSocketAddr) -> Result<()> {
        if self.addr.is_some() {
            return_errno_with_message!(Errno::EINVAL, "the socket is already bound");
        }

        let bound_addr = addr_to_bind.bind()?;
        self.addr = Some(bound_addr);

        Ok(())
    }

    pub(super) fn addr(&self) -> Option<&UnixSocketAddrBound> {
        self.addr.as_ref()
    }

    pub(super) fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
    }

    pub(super) fn register_observer(
        &self,
        observer: Weak<dyn Observer<IoEvents>>,
        mask: IoEvents,
    ) -> Result<()> {
        self.pollee.register_observer(observer, mask);
        Ok(())
    }

    pub(super) fn unregister_observer(
        &self,
        observer: &Weak<dyn Observer<IoEvents>>,
    ) -> Option<Weak<dyn Observer<IoEvents>>> {
        self.pollee.unregister_observer(observer)
    }
}
