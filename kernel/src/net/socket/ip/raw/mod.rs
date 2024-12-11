// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use aster_bigtcp::{
    socket::{SocketEventObserver, SocketEvents},
    wire::{IpAddress, IpEndpoint},
};
use ostd::sync::LocalIrqDisabled;
use takeable::Takeable;

use self::{bound::BoundRaw, unbound::UnBoundRaw};
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
            options::{Error as SocketError, IpHdrIncl, SocketOption},
            util::{
                options::{IpSocketOptionSet, SocketOptionSet},
                send_recv_flags::SendRecvFlags,
                socket_addr::SocketAddr,
                MessageHeader,
            },
            Socket,
        },
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    util::{net::Protocol, MultiRead, MultiWrite},
};

mod bound;
mod unbound;

#[derive(Debug, Clone)]
struct OptionSet {
    socket: SocketOptionSet,
    ip: IpSocketOptionSet,
}

impl OptionSet {
    fn new() -> Self {
        let socket = SocketOptionSet::new_raw();
        let ip = IpSocketOptionSet::new_raw();
        OptionSet { socket, ip }
    }
}

pub struct RawDGramSocket {
    options: RwLock<OptionSet>,
    inner: RwLock<Takeable<Inner>, LocalIrqDisabled>,
    nonblocking: AtomicBool,
    pollee: Pollee,
}

enum Inner {
    Unbound(UnBoundRaw),
    Bound(BoundRaw),
}

impl Inner {
    fn bind(
        self,
        addr: &IpAddress,
        can_reuse: bool,
    ) -> core::result::Result<BoundRaw, (Error, Self)> {
        let unbound_raw = match self {
            Inner::Unbound(unbound_raw) => unbound_raw,
            Inner::Bound(bound_raw) => {
                return Err((
                    Error::with_message(Errno::EINVAL, "the socket is already bound to an address"),
                    Inner::Bound(bound_raw),
                ));
            }
        };

        let bound_raw = match unbound_raw.bind(addr, can_reuse) {
            Ok(bound_raw) => bound_raw,
            Err((err, unbound_raw)) => return Err((err, Inner::Unbound(unbound_raw))),
        };
        Ok(bound_raw)
    }

    fn bind_to_ephemeral_endpoint(
        self,
        remote_endpoint: &IpEndpoint,
    ) -> core::result::Result<BoundRaw, (Error, Self)> {
        if let Inner::Bound(bound_raw) = self {
            return Ok(bound_raw);
        }

        let endpoint = get_ephemeral_endpoint(remote_endpoint);
        self.bind(&endpoint.addr, false)
    }
}

impl RawDGramSocket {
    pub fn new(nonblocking: bool, protocol: Protocol) -> Arc<Self> {
        Arc::new_cyclic(|me| {
            let unbound_raw = UnBoundRaw::new(me.clone() as _, protocol);
            Self {
                inner: RwLock::new(Takeable::new(Inner::Unbound(unbound_raw))),
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

    fn remote_addr(&self) -> Option<IpAddress> {
        let inner = self.inner.read();

        match inner.as_ref() {
            Inner::Bound(bound_raw) => bound_raw.remote_addr(),
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
            let bound_raw = match owned_inner.bind_to_ephemeral_endpoint(remote_endpoint) {
                Ok(bound_raw) => bound_raw,
                Err((err, err_inner)) => {
                    return (err_inner, Err(err));
                }
            };
            (Inner::Bound(bound_raw), Ok(()))
        })
    }

    fn try_recv(
        &self,
        writer: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, IpAddress)> {
        let inner = self.inner.read();

        let Inner::Bound(bound_raw) = inner.as_ref() else {
            return_errno_with_message!(Errno::EAGAIN, "the socket is not bound");
        };

        let received = bound_raw.try_recv(writer, flags);
        self.pollee.invalidate();

        received
    }

    fn recv(
        &self,
        writer: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, IpAddress)> {
        if self.is_nonblocking() {
            self.try_recv(writer, flags)
        } else {
            self.wait_events(IoEvents::IN, None, || self.try_recv(writer, flags))
        }
    }

    fn try_send(
        &self,
        reader: &mut dyn MultiRead,
        remote: &IpAddress,
        flags: SendRecvFlags,
        hdr_incl: IpHdrIncl,
    ) -> Result<usize> {
        let inner = self.inner.read();

        let Inner::Bound(bound_raw) = inner.as_ref() else {
            return_errno_with_message!(Errno::EAGAIN, "the socket is not bound")
        };

        let sent_bytes = bound_raw.try_send(reader, remote, flags, hdr_incl)?;
        let iface_to_poll = bound_raw.iface().clone();

        drop(inner);
        self.pollee.invalidate();
        iface_to_poll.poll();

        Ok(sent_bytes)
    }

    fn check_io_events(&self) -> IoEvents {
        let inner = self.inner.read();

        match inner.as_ref() {
            Inner::Unbound(unbound_raw) => unbound_raw.check_io_events(),
            Inner::Bound(bound_socket) => bound_socket.check_io_events(),
        }
    }
}

impl Pollable for RawDGramSocket {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }
}

impl FileLike for RawDGramSocket {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        // TODO: set correct flags
        let flags = SendRecvFlags::empty();
        let (read_len, _) = self.recv(writer, flags)?;
        Ok(read_len)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        let remote = self.remote_addr().ok_or_else(|| {
            Error::with_message(
                Errno::EDESTADDRREQ,
                "the destination address is not specified",
            )
        })?;

        // TODO: Set correct flags
        let flags = SendRecvFlags::empty();

        // TODO: Block if send buffer is full
        self.try_send(reader, &remote, flags, IpHdrIncl::new())
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
        // TODO: Add "SockFS" and link `RawDGramSocket` to it.
        Metadata::new_socket(
            0,
            InodeMode::from_bits_truncate(0o140777),
            aster_block::BLOCK_SIZE,
        )
    }
}

impl Socket for RawDGramSocket {
    fn bind(&self, socket_addr: SocketAddr) -> Result<()> {
        let endpoint: IpEndpoint = socket_addr.try_into()?;

        let can_reuse = self.options.read().socket.reuse_addr();
        let mut inner = self.inner.write();
        inner.borrow_result(|owned_inner| {
            let bound_raw = match owned_inner.bind(&endpoint.addr, can_reuse) {
                Ok(bound_raw) => bound_raw,
                Err((err, err_inner)) => {
                    return (err_inner, Err(err));
                }
            };
            (Inner::Bound(bound_raw), Ok(()))
        })
    }

    fn connect(&self, socket_addr: SocketAddr) -> Result<()> {
        let endpoint = socket_addr.try_into()?;

        self.try_bind_ephemeral(&endpoint)?;

        let mut inner = self.inner.write();
        let Inner::Bound(bound_raw) = inner.as_mut() else {
            return_errno_with_message!(Errno::EINVAL, "the socket is not bound")
        };
        bound_raw.set_remote_addr(&endpoint.addr);

        Ok(())
    }

    fn addr(&self) -> Result<SocketAddr> {
        let inner = self.inner.read();
        match inner.as_ref() {
            Inner::Unbound(_) => Ok(UNSPECIFIED_LOCAL_ENDPOINT.into()),
            Inner::Bound(bound_raw) => Ok(bound_raw.local_endpoint().into()),
        }
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        self.remote_addr()
            .map(|addr| IpEndpoint::new(addr, 0).into())
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
            None => {
                let ip_addr: IpEndpoint = match self.addr() {
                    Ok(ip_addr) => ip_addr.try_into()?,
                    Err(_) => {
                        return Err(Error::with_message(
                            Errno::EDESTADDRREQ,
                            "the destination address is not specified",
                        ))
                    }
                };
                IpEndpoint::new(ip_addr.addr, 0)
            }
        };

        if control_message.is_some() {
            // TODO: Support sending control message
            warn!("sending control message is not supported");
        }

        let mut op = IpHdrIncl::new();
        let _ = self.get_option(&mut op);

        // TODO: Block if the send buffer is full
        self.try_send(reader, &remote_endpoint.addr, flags, op)
    }

    fn recvmsg(
        &self,
        writer: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, MessageHeader)> {
        // TODO: Deal with flags
        debug_assert!(flags.is_all_supported());

        // This is a temporary solution. Bind to a specific iface: iface[0]
        // TODO: Monitor multiple ifaces
        let bind_iface_addr = IpEndpoint::new(IpAddress::v4(10, 0, 2, 15), 0);
        self.try_bind_ephemeral(&bind_iface_addr)?;

        let (received_bytes, addr) = self.recv(writer, flags)?;
        // TODO: Receive control message
        let IpAddress::Ipv4(ip_addr) = addr;
        let message_header = MessageHeader::new(Some(SocketAddr::IPv4(ip_addr, 0)), None);
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
        let _ = self.options.read().socket.get_option(option);
        self.options.read().ip.get_option(option)
    }

    fn set_option(&self, option: &dyn SocketOption) -> Result<()> {
        if self.options.write().socket.set_option(option).is_ok() {
            return Ok(());
        }
        if self.options.write().ip.set_option(option).is_ok() {
            Ok(())
        } else {
            return_errno_with_message!(Errno::ENOPROTOOPT, "the option to be set is unknown");
        }
    }
}

impl SocketEventObserver for RawDGramSocket {
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
