// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use aster_rights::ReadOp;

use super::{
    connected::Connected,
    listener::Listener,
    socket::{SHUT_READ_EVENTS, SHUT_WRITE_EVENTS},
};
use crate::{
    events::IoEvents,
    fs::utils::EndpointState,
    net::socket::{
        unix::{
            addr::{UnixSocketAddr, UnixSocketAddrBound},
            cred::SocketCred,
        },
        util::{options::SocketOptionSet, SockShutdownCmd},
    },
    prelude::*,
    process::signal::Pollee,
};

pub(super) struct Init {
    addr: Option<UnixSocketAddrBound>,
    is_read_shutdown: AtomicBool,
    is_write_shutdown: AtomicBool,
}

impl Init {
    pub(super) fn new() -> Self {
        Self {
            addr: None,
            is_read_shutdown: AtomicBool::new(false),
            is_write_shutdown: AtomicBool::new(false),
        }
    }

    pub(super) fn bind(&mut self, addr_to_bind: UnixSocketAddr) -> Result<()> {
        if self.addr.is_some() {
            return addr_to_bind.bind_unnamed();
        }

        let bound_addr = addr_to_bind.bind()?;
        self.addr = Some(bound_addr);

        Ok(())
    }

    pub(super) fn into_connected(
        self,
        peer_addr: UnixSocketAddrBound,
        pollee: Pollee,
        peer_cred: SocketCred,
        options: &SocketOptionSet,
    ) -> (Connected, Connected) {
        let Init {
            addr,
            is_read_shutdown,
            is_write_shutdown,
        } = self;

        pollee.invalidate();

        let cred = SocketCred::<ReadOp>::new_current();
        let (this_conn, peer_conn) = Connected::new_pair(
            addr,
            Some(peer_addr),
            EndpointState::new(pollee, is_read_shutdown.into_inner()),
            EndpointState::new(Pollee::new(), is_write_shutdown.into_inner()),
            cred,
            peer_cred,
            options,
        );

        (this_conn, peer_conn)
    }

    pub(super) fn listen(
        self,
        backlog: usize,
        pollee: Pollee,
        is_seqpacket: bool,
    ) -> core::result::Result<Listener, (Error, Self)> {
        let Some(addr) = self.addr else {
            return Err((
                Error::with_message(Errno::EINVAL, "the socket is not bound"),
                self,
            ));
        };

        pollee.invalidate();

        Ok(Listener::new(
            addr,
            backlog,
            self.is_read_shutdown.into_inner(),
            self.is_write_shutdown.into_inner(),
            pollee,
            is_seqpacket,
        ))
    }

    pub(super) fn shutdown(&self, cmd: SockShutdownCmd, pollee: &Pollee) {
        if cmd.shut_read() {
            self.is_read_shutdown.store(true, Ordering::Relaxed);
            pollee.notify(SHUT_READ_EVENTS);
        }

        if cmd.shut_write() {
            self.is_write_shutdown.store(true, Ordering::Relaxed);
            pollee.notify(SHUT_WRITE_EVENTS);
        }
    }

    pub(super) fn addr(&self) -> Option<&UnixSocketAddrBound> {
        self.addr.as_ref()
    }

    pub(super) fn is_read_shutdown(&self) -> bool {
        self.is_read_shutdown.load(Ordering::Relaxed)
    }

    pub(super) fn is_write_shutdown(&self) -> bool {
        self.is_write_shutdown.load(Ordering::Relaxed)
    }

    pub(super) fn check_io_events(&self) -> IoEvents {
        // According to the Linux implementation, we always have `IoEvents::HUP` and
        // `IoEvents::HUP` in this state.
        IoEvents::OUT | IoEvents::HUP
    }
}
