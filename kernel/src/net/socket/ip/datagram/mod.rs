// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use aster_bigtcp::{
    socket::{SocketEventObserver, SocketEvents},
    wire::IpEndpoint,
};
use ostd::sync::LocalIrqDisabled;
use takeable::Takeable;

use self::{bound::BoundDatagram, unbound::UnboundDatagram};
use super::{common::get_ephemeral_endpoint, UNSPECIFIED_LOCAL_ENDPOINT};
use crate::{
    events::IoEvents,
    fs::{
        file_handle::FileLike,
        utils::{InodeMode, Metadata, StatusFlags},
    },
    match_sock_option_mut,
    net::{
        iface::IfaceEx,
        socket::{
            options::{Error as SocketError, SocketOption},
            util::{
                options::SocketOptionSet, send_recv_flags::SendRecvFlags, socket_addr::SocketAddr,
                MessageHeader,
            },
            Socket,
        },
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    util::{MultiRead, MultiWrite},
};

mod bound;
mod unbound;

#[derive(Debug, Clone)]
struct OptionSet {
    socket: SocketOptionSet,
    // TODO: UDP option set
}

impl OptionSet {
    fn new() -> Self {
        let socket = SocketOptionSet::new_udp();
        OptionSet { socket }
    }
}

pub struct DatagramSocket {
    options: RwLock<OptionSet>,
    inner: RwLock<Takeable<Inner>, LocalIrqDisabled>,
    nonblocking: AtomicBool,
    pollee: Pollee,
}

enum Inner {
    Unbound(UnboundDatagram),
    Bound(BoundDatagram),
}

impl Inner {
    fn bind(
        self,
        endpoint: &IpEndpoint,
        can_reuse: bool,
    ) -> core::result::Result<BoundDatagram, (Error, Self)> {
        let unbound_datagram = match self {
            Inner::Unbound(unbound_datagram) => unbound_datagram,
            Inner::Bound(bound_datagram) => {
                return Err((
                    Error::with_message(Errno::EINVAL, "the socket is already bound to an address"),
                    Inner::Bound(bound_datagram),
                ));
            }
        };

        let bound_datagram = match unbound_datagram.bind(endpoint, can_reuse) {
            Ok(bound_datagram) => bound_datagram,
            Err((err, unbound_datagram)) => return Err((err, Inner::Unbound(unbound_datagram))),
        };
        Ok(bound_datagram)
    }

    fn bind_to_ephemeral_endpoint(
        self,
        remote_endpoint: &IpEndpoint,
    ) -> core::result::Result<BoundDatagram, (Error, Self)> {
        if let Inner::Bound(bound_datagram) = self {
            return Ok(bound_datagram);
        }

        let endpoint = get_ephemeral_endpoint(remote_endpoint);
        self.bind(&endpoint, false)
    }
}

impl DatagramSocket {
    pub fn new(nonblocking: bool) -> Arc<Self> {
        Arc::new_cyclic(|me| {
            let unbound_datagram = UnboundDatagram::new(me.clone() as _);
            Self {
                inner: RwLock::new(Takeable::new(Inner::Unbound(unbound_datagram))),
                nonblocking: AtomicBool::new(nonblocking),
                pollee: Pollee::new(),
                options: RwLock::new(OptionSet::new()),
            }
        })
    }

    pub fn is_nonblocking(&self) -> bool {
        self.nonblocking.load(Ordering::SeqCst)
    }

    pub fn set_nonblocking(&self, nonblocking: bool) {
        self.nonblocking.store(nonblocking, Ordering::SeqCst);
    }

    fn remote_endpoint(&self) -> Option<IpEndpoint> {
        let inner = self.inner.read();

        match inner.as_ref() {
            Inner::Bound(bound_datagram) => bound_datagram.remote_endpoint(),
            Inner::Unbound(_) => None,
        }
    }

    fn try_bind_ephemeral(&self, remote_endpoint: &IpEndpoint) -> Result<()> {
        // Fast path
        if let Inner::Bound(_) = self.inner.read().as_ref() {
            return Ok(());
        }

        // Slow path
        let mut inner = self.inner.write();
        inner.borrow_result(|owned_inner| {
            let bound_datagram = match owned_inner.bind_to_ephemeral_endpoint(remote_endpoint) {
                Ok(bound_datagram) => bound_datagram,
                Err((err, err_inner)) => {
                    return (err_inner, Err(err));
                }
            };
            (Inner::Bound(bound_datagram), Ok(()))
        })
    }

    fn try_recv(
        &self,
        writer: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, SocketAddr)> {
        let inner = self.inner.read();

        let Inner::Bound(bound_datagram) = inner.as_ref() else {
            return_errno_with_message!(Errno::EAGAIN, "the socket is not bound");
        };

        let recv_bytes = bound_datagram
            .try_recv(writer, flags)
            .map(|(recv_bytes, remote_endpoint)| (recv_bytes, remote_endpoint.into()))?;
        self.pollee.invalidate();

        Ok(recv_bytes)
    }

    fn recv(
        &self,
        writer: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, SocketAddr)> {
        if self.is_nonblocking() {
            self.try_recv(writer, flags)
        } else {
            self.wait_events(IoEvents::IN, None, || self.try_recv(writer, flags))
        }
    }

    fn try_send(
        &self,
        reader: &mut dyn MultiRead,
        remote: &IpEndpoint,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        let inner = self.inner.read();

        let Inner::Bound(bound_datagram) = inner.as_ref() else {
            return_errno_with_message!(Errno::EAGAIN, "the socket is not bound")
        };

        let sent_bytes = bound_datagram.try_send(reader, remote, flags)?;
        let iface_to_poll = bound_datagram.iface().clone();

        drop(inner);
        self.pollee.invalidate();
        iface_to_poll.poll();

        Ok(sent_bytes)
    }

    fn check_io_events(&self) -> IoEvents {
        let inner = self.inner.read();

        match inner.as_ref() {
            Inner::Unbound(unbound_datagram) => unbound_datagram.check_io_events(),
            Inner::Bound(bound_socket) => bound_socket.check_io_events(),
        }
    }
}

impl Pollable for DatagramSocket {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }
}

impl FileLike for DatagramSocket {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        // TODO: set correct flags
        let flags = SendRecvFlags::empty();
        let read_len = self.recv(writer, flags).map(|(len, _)| len)?;
        Ok(read_len)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        let remote = self.remote_endpoint().ok_or_else(|| {
            Error::with_message(
                Errno::EDESTADDRREQ,
                "the destination address is not specified",
            )
        })?;

        // TODO: Set correct flags
        let flags = SendRecvFlags::empty();

        // TODO: Block if send buffer is full
        self.try_send(reader, &remote, flags)
    }

    fn as_socket(self: Arc<Self>) -> Option<Arc<dyn Socket>> {
        Some(self)
    }

    fn status_flags(&self) -> StatusFlags {
        // TODO: when we fully support O_ASYNC, return the flag
        if self.is_nonblocking() {
            StatusFlags::O_NONBLOCK
        } else {
            StatusFlags::empty()
        }
    }

    fn set_status_flags(&self, new_flags: StatusFlags) -> Result<()> {
        if new_flags.contains(StatusFlags::O_NONBLOCK) {
            self.set_nonblocking(true);
        } else {
            self.set_nonblocking(false);
        }
        Ok(())
    }

    fn metadata(&self) -> Metadata {
        // This is a dummy implementation.
        // TODO: Add "SockFS" and link `DatagramSocket` to it.
        Metadata::new_socket(
            0,
            InodeMode::from_bits_truncate(0o140777),
            aster_block::BLOCK_SIZE,
        )
    }
}

impl Socket for DatagramSocket {
    fn bind(&self, socket_addr: SocketAddr) -> Result<()> {
        let endpoint = socket_addr.try_into()?;

        let can_reuse = self.options.read().socket.reuse_addr();
        let mut inner = self.inner.write();
        inner.borrow_result(|owned_inner| {
            let bound_datagram = match owned_inner.bind(&endpoint, can_reuse) {
                Ok(bound_datagram) => bound_datagram,
                Err((err, err_inner)) => {
                    return (err_inner, Err(err));
                }
            };
            (Inner::Bound(bound_datagram), Ok(()))
        })
    }

    fn connect(&self, socket_addr: SocketAddr) -> Result<()> {
        let endpoint = socket_addr.try_into()?;

        self.try_bind_ephemeral(&endpoint)?;

        let mut inner = self.inner.write();
        let Inner::Bound(bound_datagram) = inner.as_mut() else {
            return_errno_with_message!(Errno::EINVAL, "the socket is not bound")
        };
        bound_datagram.set_remote_endpoint(&endpoint);

        Ok(())
    }

    fn addr(&self) -> Result<SocketAddr> {
        let inner = self.inner.read();
        match inner.as_ref() {
            Inner::Unbound(_) => Ok(UNSPECIFIED_LOCAL_ENDPOINT.into()),
            Inner::Bound(bound_datagram) => Ok(bound_datagram.local_endpoint().into()),
        }
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        self.remote_endpoint()
            .map(|endpoint| endpoint.into())
            .ok_or_else(|| Error::with_message(Errno::ENOTCONN, "the socket is not connected"))
    }

    fn sendmsg(
        &self,
        reader: &mut dyn MultiRead,
        message_header: MessageHeader,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        // TODO: Deal with flags
        debug_assert!(flags.is_all_supported());

        let MessageHeader {
            addr,
            control_message,
        } = message_header;

        let remote_endpoint = match addr {
            Some(remote_addr) => {
                let endpoint = remote_addr.try_into()?;
                self.try_bind_ephemeral(&endpoint)?;
                endpoint
            }
            None => self.remote_endpoint().ok_or_else(|| {
                Error::with_message(
                    Errno::EDESTADDRREQ,
                    "the destination address is not specified",
                )
            })?,
        };

        if control_message.is_some() {
            // TODO: Support sending control message
            warn!("sending control message is not supported");
        }

        // TODO: Block if the send buffer is full
        self.try_send(reader, &remote_endpoint, flags)
    }

    fn recvmsg(
        &self,
        writer: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, MessageHeader)> {
        // TODO: Deal with flags
        debug_assert!(flags.is_all_supported());

        let (received_bytes, peer_addr) = self.recv(writer, flags)?;

        // TODO: Receive control message

        let message_header = MessageHeader::new(Some(peer_addr), None);

        Ok((received_bytes, message_header))
    }

    fn get_option(&self, option: &mut dyn SocketOption) -> Result<()> {
        match_sock_option_mut!(option, {
            socket_errors: SocketError => {
                self.options.write().socket.get_and_clear_sock_errors(socket_errors);
                return Ok(());
            },
            _ => ()
        });

        self.options.read().socket.get_option(option)
    }

    fn set_option(&self, option: &dyn SocketOption) -> Result<()> {
        self.options.write().socket.set_option(option)
    }
}

impl SocketEventObserver for DatagramSocket {
    fn on_events(&self, events: SocketEvents) {
        let mut io_events = IoEvents::empty();

        if events.contains(SocketEvents::CAN_RECV) {
            io_events |= IoEvents::IN;
        }

        if events.contains(SocketEvents::CAN_SEND) {
            io_events |= IoEvents::OUT;
        }

        self.pollee.notify(io_events);
    }
}
