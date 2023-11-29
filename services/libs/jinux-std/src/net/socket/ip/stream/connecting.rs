use core::sync::atomic::{AtomicBool, Ordering};

use alloc::sync::Arc;

use crate::events::IoEvents;
use crate::net::poll_ifaces;
use crate::prelude::*;

use crate::net::iface::{AnyBoundSocket, IpEndpoint};
use crate::process::signal::Poller;

use super::connected::ConnectedStream;
use super::init::InitStream;

pub struct ConnectingStream {
    nonblocking: AtomicBool,
    bound_socket: Arc<AnyBoundSocket>,
    remote_endpoint: IpEndpoint,
}

impl ConnectingStream {
    pub fn new(
        nonblocking: bool,
        bound_socket: Arc<AnyBoundSocket>,
        remote_endpoint: IpEndpoint,
    ) -> Result<Self> {
        bound_socket.do_connect(remote_endpoint)?;

        Ok(Self {
            nonblocking: AtomicBool::new(nonblocking),
            bound_socket,
            remote_endpoint,
        })
    }

    pub fn wait_conn(&self) -> core::result::Result<ConnectedStream, (Error, InitStream)> {
        debug_assert!(!self.is_nonblocking());

        let poller = Poller::new();
        loop {
            poll_ifaces();

            let events = self.poll(IoEvents::OUT | IoEvents::IN, Some(&poller));
            if events.contains(IoEvents::IN) || events.contains(IoEvents::OUT) {
                return Ok(ConnectedStream::new(
                    self.is_nonblocking(),
                    self.bound_socket.clone(),
                    self.remote_endpoint,
                ));
            } else if !events.is_empty() {
                return Err((
                    Error::with_message(Errno::ECONNREFUSED, "connection refused"),
                    InitStream::new_bound(self.is_nonblocking(), self.bound_socket.clone()),
                ));
            } else {
                // FIXME: deal with nonblocking mode & connecting timeout
                poller.wait().expect("async connect() not implemented");
            }
        }
    }

    pub fn local_endpoint(&self) -> Result<IpEndpoint> {
        self.bound_socket
            .local_endpoint()
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "no local endpoint"))
    }

    pub fn remote_endpoint(&self) -> Result<IpEndpoint> {
        Ok(self.remote_endpoint)
    }

    pub fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.bound_socket.poll(mask, poller)
    }

    pub fn is_nonblocking(&self) -> bool {
        self.nonblocking.load(Ordering::Relaxed)
    }

    pub fn set_nonblocking(&self, nonblocking: bool) {
        self.nonblocking.store(nonblocking, Ordering::Relaxed);
    }
}
