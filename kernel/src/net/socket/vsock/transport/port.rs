// SPDX-License-Identifier: MPL-2.0

use alloc::collections::btree_map::BTreeMap;

use crate::{
    error::{Errno, Error, return_errno_with_message},
    net::socket::vsock::{
        addr::{VMADDR_CID_ANY, VMADDR_PORT_ANY, VsockSocketAddr},
        transport::{
            Connection, Listener,
            space::{VsockSpace, vsock_space},
        },
    },
    prelude::Result,
    process::signal::Pollee,
};

/// An owned lease on a bound local vsock port; dropping the value releases the lease.
//
// TODO: Currently, the port table operates globally and checks whether a port is occupied without
// taking the CID into account. This only works correctly if there is one vsock device.
#[derive(Debug)]
pub(in crate::net::socket::vsock) struct BoundPort {
    port: u32,
}

pub(super) struct PortTable {
    next_ephemeral_port: u32,
    usage: BTreeMap<u32, usize>,
}

impl PortTable {
    const EPHEMERAL_PORT_START: u32 = 49152;

    pub(super) fn new() -> Self {
        Self {
            next_ephemeral_port: Self::EPHEMERAL_PORT_START,
            usage: BTreeMap::new(),
        }
    }

    fn next_ephemeral_port_after(port: u32) -> u32 {
        let mut next_port = if port == u32::MAX {
            Self::EPHEMERAL_PORT_START
        } else {
            port + 1
        };
        if next_port < Self::EPHEMERAL_PORT_START || next_port == VMADDR_PORT_ANY {
            next_port = Self::EPHEMERAL_PORT_START;
        }
        next_port
    }
}

impl BoundPort {
    /// Binds exclusively to `addr` and returns the resulting port lease.
    pub(in crate::net::socket::vsock) fn new_exclusive(addr: VsockSocketAddr) -> Result<Self> {
        let vsock_space = vsock_space()?;

        let guest_cid = vsock_space.guest_cid();
        if addr.cid != VMADDR_CID_ANY && addr.cid as u64 != guest_cid {
            return_errno_with_message!(Errno::EADDRNOTAVAIL, "the vsock CID is not local");
        }

        if addr.port == VMADDR_PORT_ANY {
            return Self::new_ephemeral();
        }

        let mut ports = vsock_space.lock_ports();
        let usage = ports.usage.entry(addr.port).or_insert(0);
        if *usage != 0 {
            return_errno_with_message!(Errno::EADDRINUSE, "the vsock port is already in use");
        }
        *usage += 1;
        Ok(Self { port: addr.port })
    }

    /// Allocates and returns a fresh ephemeral port lease.
    pub(in crate::net::socket::vsock) fn new_ephemeral() -> Result<Self> {
        let vsock_space = vsock_space()?;
        let mut ports = vsock_space.lock_ports();

        let start_port = ports.next_ephemeral_port;
        let mut current_port = start_port;

        loop {
            let usage = ports.usage.entry(current_port).or_insert(0);
            if *usage == 0 {
                *usage += 1;
                ports.next_ephemeral_port = PortTable::next_ephemeral_port_after(current_port);
                return Ok(Self { port: current_port });
            }

            current_port = PortTable::next_ephemeral_port_after(current_port);
            if current_port == start_port {
                return_errno_with_message!(
                    Errno::EADDRINUSE,
                    "no ephemeral vsock ports are available"
                );
            }
        }
    }

    pub(super) fn new_shared(bound_port: &BoundPort) -> BoundPort {
        let vsock_space = bound_port.vsock_space();

        let mut ports = vsock_space.lock_ports();
        let usage = ports.usage.entry(bound_port.port).or_insert(0);
        *usage += 1;
        BoundPort {
            port: bound_port.port,
        }
    }

    /// Starts a connection attempt to `remote_addr`.
    ///
    /// On success, ownership of the lease moves into the returned `Connection`. On failure, the
    /// error is returned together with the original lease.
    pub(in crate::net::socket::vsock) fn connect(
        self,
        remote_addr: VsockSocketAddr,
        pollee: &Pollee,
    ) -> core::result::Result<Connection, (Error, BoundPort)> {
        let vsock_space = self.vsock_space();
        vsock_space.new_connection(self, remote_addr, pollee)
    }

    /// Starts listening on the leased port.
    ///
    /// On success, ownership of the lease moves into the returned `Listener`. On failure, the
    /// error is returned together with the original lease.
    pub(in crate::net::socket::vsock) fn listen(
        self,
        backlog: usize,
        pollee: &Pollee,
    ) -> core::result::Result<Listener, (Error, BoundPort)> {
        let vsock_space = self.vsock_space();
        vsock_space.new_listener(self, backlog, pollee)
    }

    /// Returns the local address described by this lease.
    pub(in crate::net::socket::vsock) fn local_addr(&self) -> VsockSocketAddr {
        VsockSocketAddr {
            cid: self.vsock_space().guest_cid() as u32,
            port: self.port,
        }
    }

    pub(super) fn vsock_space(&self) -> &'static VsockSpace {
        // This won't fail because we've checked it in all constructors.
        vsock_space().unwrap()
    }

    pub(super) const fn port(&self) -> u32 {
        self.port
    }
}

impl Drop for BoundPort {
    fn drop(&mut self) {
        use alloc::collections::btree_map::Entry;

        let mut ports = self.vsock_space().lock_ports();
        let Entry::Occupied(mut usage) = ports.usage.entry(self.port) else {
            return;
        };
        *usage.get_mut() -= 1;
        if *usage.get() == 0 {
            usage.remove();
        }
    }
}
