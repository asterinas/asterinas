use core::sync::atomic::{AtomicBool, Ordering};

use alloc::sync::Arc;

use crate::events::{IoEvents, Observer};
use crate::net::iface::RawTcpSocket;
use crate::net::poll_ifaces;
use crate::prelude::*;

use crate::net::iface::{AnyBoundSocket, IpEndpoint};
use crate::process::signal::{Pollee, Poller};

use super::connected::ConnectedStream;
use super::init::InitStream;

pub struct ConnectingStream {
    nonblocking: AtomicBool,
    bound_socket: Arc<AnyBoundSocket>,
    remote_endpoint: IpEndpoint,
    conn_result: RwLock<Option<ConnResult>>,
    pollee: Pollee,
}

enum ConnResult {
    Connected,
    Refused,
}

impl ConnectingStream {
    pub fn new(
        nonblocking: bool,
        bound_socket: Arc<AnyBoundSocket>,
        remote_endpoint: IpEndpoint,
        pollee: Pollee,
    ) -> Result<Arc<Self>> {
        bound_socket.do_connect(remote_endpoint)?;

        let connecting = Arc::new(Self {
            nonblocking: AtomicBool::new(nonblocking),
            bound_socket,
            remote_endpoint,
            conn_result: RwLock::new(None),
            pollee,
        });
        connecting.pollee.reset_events();
        connecting
            .bound_socket
            .set_observer(Arc::downgrade(&connecting) as _);
        Ok(connecting)
    }

    pub fn wait_conn(
        &self,
    ) -> core::result::Result<Arc<ConnectedStream>, (Error, Arc<InitStream>)> {
        debug_assert!(!self.is_nonblocking());

        let poller = Poller::new();
        loop {
            poll_ifaces();

            match *self.conn_result.read() {
                Some(ConnResult::Connected) => {
                    return Ok(ConnectedStream::new(
                        self.is_nonblocking(),
                        self.bound_socket.clone(),
                        self.remote_endpoint,
                        self.pollee.clone(),
                    ));
                }
                Some(ConnResult::Refused) => {
                    return Err((
                        Error::with_message(Errno::ECONNREFUSED, "connection refused"),
                        InitStream::new_bound(
                            self.is_nonblocking(),
                            self.bound_socket.clone(),
                            self.pollee.clone(),
                        ),
                    ));
                }
                None => (),
            };

            let events = self.poll(IoEvents::OUT, Some(&poller));
            if !events.contains(IoEvents::OUT) {
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
        self.pollee.poll(mask, poller)
    }

    pub fn is_nonblocking(&self) -> bool {
        self.nonblocking.load(Ordering::Relaxed)
    }

    pub fn set_nonblocking(&self, nonblocking: bool) {
        self.nonblocking.store(nonblocking, Ordering::Relaxed);
    }

    fn update_io_events(&self) {
        if self.conn_result.read().is_some() {
            return;
        }

        let became_writable = self.bound_socket.raw_with(|socket: &mut RawTcpSocket| {
            let mut result = self.conn_result.write();
            if result.is_some() {
                return false;
            }

            // Connected
            if socket.can_send() {
                *result = Some(ConnResult::Connected);
                return true;
            }
            // Connecting
            if socket.is_open() {
                return false;
            }
            // Refused
            *result = Some(ConnResult::Refused);
            true
        });

        // Either when the connection is established, or when the connection fails, the socket
        // shall indicate that it is writable.
        //
        // TODO: Find a way to turn `ConnectingStream` into `ConnectedStream` or `InitStream`
        // here, so non-blocking `connect()` can work correctly. Meanwhile, the latter should
        // be responsible to initialize all the I/O events including `IoEvents::OUT`, so the
        // following hard-coded event addition can be removed.
        if became_writable {
            self.pollee.add_events(IoEvents::OUT);
        }
    }
}

impl Observer<()> for ConnectingStream {
    fn on_events(&self, _: &()) {
        self.update_io_events();
    }
}
