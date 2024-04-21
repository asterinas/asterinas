// SPDX-License-Identifier: MPL-2.0

use crate::{
    events::{IoEvents, Observer},
    net::socket::vsock::{
        addr::{VsockSocketAddr, VMADDR_CID_ANY, VMADDR_PORT_ANY},
        VSOCK_GLOBAL,
    },
    prelude::*,
    process::signal::{Pollee, Poller},
};

pub struct Init {
    bind_addr: SpinLock<Option<VsockSocketAddr>>,
    pollee: Pollee,
}

impl Init {
    pub fn new() -> Self {
        Self {
            bind_addr: SpinLock::new(None),
            pollee: Pollee::new(IoEvents::empty()),
        }
    }

    pub fn bind(&self, addr: VsockSocketAddr) -> Result<()> {
        if self.bind_addr.lock().is_some() {
            return_errno_with_message!(Errno::EINVAL, "the socket is already bound");
        }
        let vsockspace = VSOCK_GLOBAL.get().unwrap();

        // check correctness of cid
        let local_cid = vsockspace.driver.lock_irq_disabled().guest_cid();
        if addr.cid != VMADDR_CID_ANY && addr.cid != local_cid as u32 {
            return_errno_with_message!(Errno::EADDRNOTAVAIL, "The cid in address is incorrect");
        }
        let mut new_addr = addr;
        new_addr.cid = local_cid as u32;

        // check and assign a port
        if addr.port == VMADDR_PORT_ANY {
            if let Ok(port) = vsockspace.alloc_ephemeral_port() {
                new_addr.port = port;
            } else {
                return_errno_with_message!(Errno::EAGAIN, "cannot find unused high port");
            }
        } else if vsockspace
            .used_ports
            .lock_irq_disabled()
            .contains(&new_addr.port)
        {
            return_errno_with_message!(Errno::EADDRNOTAVAIL, "the port in address is occupied");
        } else {
            vsockspace
                .used_ports
                .lock_irq_disabled()
                .insert(new_addr.port);
        }

        //TODO: The privileged port isn't checked
        *self.bind_addr.lock() = Some(new_addr);
        Ok(())
    }

    pub fn is_bound(&self) -> bool {
        self.bind_addr.lock().is_some()
    }

    pub fn bound_addr(&self) -> Option<VsockSocketAddr> {
        *self.bind_addr.lock()
    }

    pub fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
    }

    pub fn register_observer(
        &self,
        pollee: &Pollee,
        observer: Weak<dyn Observer<IoEvents>>,
        mask: IoEvents,
    ) -> Result<()> {
        pollee.register_observer(observer, mask);
        Ok(())
    }

    pub fn unregister_observer(
        &self,
        pollee: &Pollee,
        observer: &Weak<dyn Observer<IoEvents>>,
    ) -> Result<Weak<dyn Observer<IoEvents>>> {
        pollee
            .unregister_observer(observer)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "fails to unregister observer"))
    }
}

impl Drop for Init {
    fn drop(&mut self) {
        if let Some(addr) = *self.bind_addr.lock() {
            let vsockspace = VSOCK_GLOBAL.get().unwrap();
            vsockspace.used_ports.lock_irq_disabled().remove(&addr.port);
        }
    }
}

impl Default for Init {
    fn default() -> Self {
        Self::new()
    }
}
