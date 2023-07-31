use crate::{
    net::socket::{unix::addr::UnixSocketAddr, SockShutdownCmd},
    prelude::*,
};

use super::endpoint::Endpoint;

pub struct Connected {
    local_endpoint: Arc<Endpoint>,
    // The peer addr is None if peer is unnamed.
    // FIXME: can a socket be bound after the socket is connected?
    peer_addr: Option<UnixSocketAddr>,
}

impl Connected {
    pub fn new(local_endpoint: Arc<Endpoint>) -> Self {
        let peer_addr = local_endpoint.peer_addr();
        Connected {
            local_endpoint,
            peer_addr,
        }
    }

    pub fn addr(&self) -> Option<UnixSocketAddr> {
        self.local_endpoint.addr()
    }

    pub fn peer_addr(&self) -> Option<&UnixSocketAddr> {
        self.peer_addr.as_ref()
    }

    pub fn is_bound(&self) -> bool {
        self.addr().is_some()
    }

    pub fn write(&self, buf: &[u8]) -> Result<usize> {
        self.local_endpoint.write(buf)
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        self.local_endpoint.read(buf)
    }

    pub fn shutdown(&self, cmd: SockShutdownCmd) -> Result<()> {
        self.local_endpoint.shutdown(cmd)
    }

    pub fn is_nonblocking(&self) -> bool {
        self.local_endpoint.is_nonblocking()
    }

    pub fn set_nonblocking(&self, is_nonblocking: bool) {
        self.local_endpoint.set_nonblocking(is_nonblocking).unwrap();
    }
}
