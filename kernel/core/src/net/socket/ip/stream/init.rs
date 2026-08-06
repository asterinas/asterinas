// SPDX-License-Identifier: MPL-2.0

use core::{
    net::Ipv4Addr,
    sync::atomic::{AtomicBool, Ordering},
};

use aster_bigtcp::{
    socket::RawTcpOption,
    wire::{IpAddress, IpEndpoint, Ipv6Address},
};

use super::{connecting::ConnectingStream, listen::ListenStream, observer::StreamObserver};
use crate::{
    events::IoEvents,
    net::{
        iface::BoundTcpPort,
        socket::{
            ip::{
                addr::IpAddressFamily,
                common::{
                    get_ephemeral_endpoint, map_unspecified_to_localhost,
                    resolve_bind_iface_and_config,
                },
            },
            util::SocketAddr,
        },
    },
    prelude::*,
};

pub(super) struct InitStream {
    bound_port: Option<BoundTcpPort>,
    /// Indicates if the last `connect()` is considered to be done.
    ///
    /// If `connect()` is called but we're still in the `InitStream`, this means that the
    /// connection is refused.
    ///
    ///  * If the connection is refused synchronously, the error code is returned by the
    ///    `connect()` system call, and after that we always consider the `connect()` to be already
    ///    done.
    ///
    ///  * If the connection is refused asynchronously (e.g., non-blocking sockets or interrupted
    ///    `connect()`), the last `connect()` is not considered to have been done until another
    ///    `connect()`, which checks and resets the boolean value and returns an appropriate error
    ///    code.
    is_connect_done: bool,
    /// Indicates whether the socket error is `ECONNREFUSED`.
    ///
    /// This boolean value is set to true when the connection is refused and set to false when the
    /// error code is reported via `getsockopt(SOL_SOCKET, SO_ERROR)`, `send()`, `recv()`, or
    /// `connect()`.
    is_conn_refused: AtomicBool,
}

impl InitStream {
    pub(super) fn new() -> Self {
        Self {
            bound_port: None,
            is_connect_done: true,
            is_conn_refused: AtomicBool::new(false),
        }
    }

    pub(super) fn new_bound(bound_port: BoundTcpPort) -> Self {
        Self {
            bound_port: Some(bound_port),
            is_connect_done: true,
            is_conn_refused: AtomicBool::new(false),
        }
    }

    pub(super) fn new_refused(bound_port: BoundTcpPort) -> Self {
        Self {
            bound_port: Some(bound_port),
            is_connect_done: false,
            is_conn_refused: AtomicBool::new(true),
        }
    }

    pub(super) fn bind(
        &mut self,
        endpoint: &IpEndpoint,
        family: IpAddressFamily,
        can_reuse: bool,
    ) -> Result<()> {
        if self.bound_port.is_some() {
            return_errno_with_message!(Errno::EINVAL, "the socket is already bound to an address");
        }

        // When we support `IPV6_V6ONLY` and if it is set, we should also reject IPv4-mapped
        // IPv6 addresses.
        if IpAddressFamily::from(endpoint.addr) != family {
            return_errno_with_message!(
                Errno::EAFNOSUPPORT,
                "the protocol family does not match the address family"
            );
        }

        self.bound_port = Some(bind_port(endpoint, can_reuse)?);

        Ok(())
    }

    pub(super) fn bound_port(&self) -> Option<&BoundTcpPort> {
        self.bound_port.as_ref()
    }

    pub(super) fn connect(
        self,
        remote_endpoint: &IpEndpoint,
        family: IpAddressFamily,
        option: &RawTcpOption,
        can_reuse: bool,
        observer: StreamObserver,
    ) -> Result<ConnectingStream, (Error, Self)> {
        debug_assert!(
            self.is_connect_done,
            "`finish_last_connect()` should be called before calling `connect()`"
        );

        // When we support `IPV6_V6ONLY` and if it is set, we should also reject IPv4-mapped
        // IPv6 addresses.
        if IpAddressFamily::from(remote_endpoint.addr) != family {
            return Err((
                Error::with_message(
                    Errno::EAFNOSUPPORT,
                    "the protocol family does not match the address family",
                ),
                self,
            ));
        }

        // FIXME: This is a temporary compatibility workaround for some gVisor test cases.
        // Remove it once IPv4-mapped IPv6 TCP is supported.
        // gVisor expects that connecting to `::ffff:0.0.0.0` returns `ECONNREFUSED`
        // when there's no listener on that address.
        // Linux handles `::ffff:0.0.0.0` via the IPv4 TCP path and routes it to `127.0.0.1`.
        // IPv4-mapped loopback addresses also use the IPv4 TCP path.
        let is_ipv4_mapped_localhost = matches!(
            remote_endpoint.addr,
            IpAddress::Ipv6(address)
                if address
                    .to_ipv4_mapped()
                    .is_some_and(|address| address.is_unspecified() || address.is_loopback())
        );

        let remote_endpoint = map_unspecified_to_localhost(*remote_endpoint);
        let route_endpoint = if is_ipv4_mapped_localhost {
            IpEndpoint::new(Ipv6Address::LOCALHOST.into(), remote_endpoint.port)
        } else {
            remote_endpoint
        };

        let bound_port = if let Some(bound_port) = self.bound_port {
            bound_port
        } else {
            let endpoint = match get_ephemeral_endpoint(&route_endpoint) {
                Ok(endpoint) => endpoint,
                Err(err) => return Err((err, self)),
            };
            match bind_port(&endpoint, can_reuse) {
                Ok(bound_port) => bound_port,
                Err(err) => return Err((err, self)),
            }
        };

        if is_ipv4_mapped_localhost {
            return Err((
                Error::with_message(
                    Errno::ECONNREFUSED,
                    "IPv4-mapped IPv6 connections are not supported",
                ),
                InitStream::new_refused(bound_port),
            ));
        }

        ConnectingStream::new(bound_port, remote_endpoint, option, observer).map_err(
            |(err, bound_port)| {
                if err.error() == Errno::ECONNREFUSED {
                    (err, InitStream::new_refused(bound_port))
                } else {
                    (err, InitStream::new_bound(bound_port))
                }
            },
        )
    }

    pub(super) fn finish_last_connect(&mut self) -> Result<()> {
        if self.is_connect_done {
            return Ok(());
        }

        self.is_connect_done = true;

        let is_conn_refused = self.is_conn_refused.get_mut();
        if *is_conn_refused {
            *is_conn_refused = false;
            return_errno_with_message!(Errno::ECONNREFUSED, "the connection is refused");
        } else {
            return_errno_with_message!(
                Errno::ECONNABORTED,
                "the error code for the connection failure is not available"
            );
        }
    }

    pub(super) fn listen(
        self,
        backlog: usize,
        option: &RawTcpOption,
        observer: StreamObserver,
    ) -> Result<ListenStream, (Error, Self)> {
        if !self.is_connect_done {
            // See the comments of `is_connect_done`.
            // `listen()` is also not allowed until the second `connect()`.
            return Err((
                Error::with_message(
                    Errno::EINVAL,
                    "the connection is refused, but the connecting phase is not done",
                ),
                self,
            ));
        }

        let Some(bound_port) = self.bound_port else {
            // FIXME: The socket should be bound to INADDR_ANY (i.e., 0.0.0.0) with an ephemeral
            // port. However, INADDR_ANY is not yet supported, so we need to return an error first.
            warn!("listen() without bind() is not implemented");
            return Err((
                Error::with_message(Errno::EINVAL, "listen() without bind() is not implemented"),
                self,
            ));
        };

        match ListenStream::new(bound_port, backlog, option, observer) {
            Ok(listen_stream) => Ok(listen_stream),
            Err((bound_port, error)) => Err((error, Self::new_bound(bound_port))),
        }
    }

    pub(super) fn try_recv(&self) -> Result<(usize, SocketAddr)> {
        // FIXME: Linux does not return addresses for `recvfrom` on connection-oriented sockets.
        // This is a placeholder that has no Linux equivalent. (Note also that in this case
        // `getpeeraddr` will simply fail with `ENOTCONN`).
        const UNSPECIFIED_SOCKET_ADDR: SocketAddr = SocketAddr::IPv4(Ipv4Addr::UNSPECIFIED, 0);

        // Below are some magic checks to make our behavior identical to Linux.

        if self.is_connect_done {
            return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected");
        }

        if let Some(err) = self.test_and_clear_error() {
            return Err(err);
        }

        Ok((0, UNSPECIFIED_SOCKET_ADDR))
    }

    pub(super) fn try_send(&self) -> Result<usize> {
        if let Some(err) = self.test_and_clear_error() {
            return Err(err);
        }

        return_errno_with_message!(Errno::EPIPE, "the socket is not connected");
    }

    pub(super) fn local_endpoint(&self) -> Option<IpEndpoint> {
        self.bound_port
            .as_ref()
            .map(|bound_port| bound_port.endpoint())
    }

    pub(super) fn check_io_events(&self) -> IoEvents {
        // Linux adds OUT and HUP events for a newly created socket
        let mut events = IoEvents::OUT | IoEvents::HUP;

        if self.is_conn_refused.load(Ordering::Relaxed) {
            events |= IoEvents::ERR;
        }

        events
    }

    pub(super) fn test_and_clear_error(&self) -> Option<Error> {
        if self.is_conn_refused.swap(false, Ordering::Relaxed) {
            Some(Error::with_message(
                Errno::ECONNREFUSED,
                "the connection is refused",
            ))
        } else {
            None
        }
    }
}

fn bind_port(endpoint: &IpEndpoint, can_reuse: bool) -> Result<BoundTcpPort> {
    let (iface, config) = resolve_bind_iface_and_config(endpoint, can_reuse)?;
    Ok(iface.bind_tcp(config)?)
}
