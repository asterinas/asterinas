// SPDX-License-Identifier: MPL-2.0

use crate::{
    events::IoEvents,
    net::socket::vsock::{
        addr::{VsockSocketAddr, VMADDR_CID_ANY, VMADDR_PORT_ANY},
        VSOCK_GLOBAL,
    },
    prelude::*,
    process::signal::PollHandle,
};

pub struct Init {
    bound_addr: Mutex<Option<VsockSocketAddr>>,
}

impl Init {
    pub fn new() -> Self {
        Self {
            bound_addr: Mutex::new(None),
        }
    }

    pub fn bind(&self, addr: VsockSocketAddr) -> Result<()> {
        if self.bound_addr.lock().is_some() {
            return_errno_with_message!(Errno::EINVAL, "the socket is already bound");
        }
        let vsockspace = VSOCK_GLOBAL.get().unwrap();

        // check correctness of cid
        let local_cid = vsockspace.guest_cid();
        if addr.cid != VMADDR_CID_ANY && addr.cid != local_cid {
            return_errno_with_message!(Errno::EADDRNOTAVAIL, "the cid in address is incorrect");
        }
        let mut new_addr = addr;
        new_addr.cid = local_cid;

        // check and assign a port
        if addr.port == VMADDR_PORT_ANY {
            if let Ok(port) = vsockspace.alloc_ephemeral_port() {
                new_addr.port = port;
            } else {
                return_errno_with_message!(Errno::EAGAIN, "cannot find unused high port");
            }
        } else if !vsockspace.bind_port(new_addr.port) {
            return_errno_with_message!(Errno::EADDRNOTAVAIL, "the port in address is occupied");
        }

        //TODO: The privileged port isn't checked
        *self.bound_addr.lock() = Some(new_addr);
        Ok(())
    }

    pub fn is_bound(&self) -> bool {
        self.bound_addr.lock().is_some()
    }

    pub fn bound_addr(&self) -> Option<VsockSocketAddr> {
        *self.bound_addr.lock()
    }

    pub fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        IoEvents::empty()
    }
}

impl Default for Init {
    fn default() -> Self {
        Self::new()
    }
}
