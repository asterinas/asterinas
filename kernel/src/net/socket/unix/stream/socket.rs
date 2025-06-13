// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use takeable::Takeable;

use super::{
    connected::Connected,
    init::Init,
    listener::{get_backlog, Backlog, Listener},
};
use crate::{
    events::IoEvents,
    fs::{file_handle::FileLike, utils::EndpointState},
    net::socket::{
        private::SocketPrivate,
        unix::UnixSocketAddr,
        util::{MessageHeader, SendRecvFlags, SockShutdownCmd, SocketAddr},
        Socket,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    util::{MultiRead, MultiWrite},
};

pub struct UnixStreamSocket {
    state: RwMutex<Takeable<State>>,

    pollee: Pollee,
    is_nonblocking: AtomicBool,
}

impl UnixStreamSocket {
    pub(super) fn new_init(init: Init, is_nonblocking: bool) -> Arc<Self> {
        Arc::new(Self {
            state: RwMutex::new(Takeable::new(State::Init(init))),
            pollee: Pollee::new(),
            is_nonblocking: AtomicBool::new(is_nonblocking),
        })
    }

    pub(super) fn new_connected(connected: Connected, is_nonblocking: bool) -> Arc<Self> {
        let cloned_pollee = connected.cloned_pollee();
        Arc::new(Self {
            state: RwMutex::new(Takeable::new(State::Connected(connected))),
            pollee: cloned_pollee,
            is_nonblocking: AtomicBool::new(is_nonblocking),
        })
    }
}

enum State {
    Init(Init),
    Listen(Listener),
    Connected(Connected),
}

impl State {
    pub(self) fn check_io_events(&self) -> IoEvents {
        let mut events = IoEvents::empty();

        let is_read_shutdown = self.is_read_shutdown();
        let is_write_shutdown = self.is_write_shutdown();

        if is_read_shutdown {
            // The socket is shut down in one direction: the remote socket has shut down for
            // writing or the local socket has shut down for reading.
            events |= IoEvents::RDHUP | IoEvents::IN;

            if is_write_shutdown {
                // The socket is shut down in both directions. Neither reading nor writing is
                // possible.
                events |= IoEvents::HUP;
            }
        }

        if is_write_shutdown && !matches!(self, State::Listen(_)) {
            // The socket is shut down in another direction: The remote socket has shut down for
            // reading or the local socket has shut down for writing.
            events |= IoEvents::OUT;
        }

        events |= match self {
            State::Init(init) => init.check_io_events(),
            State::Listen(listener) => listener.check_io_events(),
            State::Connected(connected) => connected.check_io_events(),
        };

        events
    }

    fn is_read_shutdown(&self) -> bool {
        match self {
            State::Init(init) => init.is_read_shutdown(),
            State::Listen(listener) => listener.is_read_shutdown(),
            State::Connected(connected) => connected.is_read_shutdown(),
        }
    }

    fn is_write_shutdown(&self) -> bool {
        match self {
            State::Init(init) => init.is_write_shutdown(),
            State::Listen(listener) => listener.is_write_shutdown(),
            State::Connected(connected) => connected.is_write_shutdown(),
        }
    }
}

impl UnixStreamSocket {
    pub fn new(is_nonblocking: bool) -> Arc<Self> {
        Self::new_init(Init::new(), is_nonblocking)
    }

    pub fn new_pair(is_nonblocking: bool) -> (Arc<Self>, Arc<Self>) {
        let (conn_a, conn_b) = Connected::new_pair(
            None,
            None,
            EndpointState::default(),
            EndpointState::default(),
        );
        (
            Self::new_connected(conn_a, is_nonblocking),
            Self::new_connected(conn_b, is_nonblocking),
        )
    }

    fn try_send(&self, buf: &mut dyn MultiRead, _flags: SendRecvFlags) -> Result<usize> {
        match self.state.read().as_ref() {
            State::Connected(connected) => connected.try_write(buf),
            State::Init(_) | State::Listen(_) => {
                return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected")
            }
        }
    }

    fn try_recv(&self, buf: &mut dyn MultiWrite, _flags: SendRecvFlags) -> Result<usize> {
        match self.state.read().as_ref() {
            State::Connected(connected) => connected.try_read(buf),
            State::Init(_) | State::Listen(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is not connected")
            }
        }
    }

    fn try_connect(&self, backlog: &Arc<Backlog>) -> Result<()> {
        let mut state = self.state.write();

        state.borrow_result(|owned_state| {
            let init = match owned_state {
                State::Init(init) => init,
                State::Listen(listener) => {
                    return (
                        State::Listen(listener),
                        Err(Error::with_message(
                            Errno::EINVAL,
                            "the socket is listening",
                        )),
                    );
                }
                State::Connected(connected) => {
                    return (
                        State::Connected(connected),
                        Err(Error::with_message(
                            Errno::EISCONN,
                            "the socket is connected",
                        )),
                    );
                }
            };

            let connected = match backlog.push_incoming(init, self.pollee.clone()) {
                Ok(connected) => connected,
                Err((err, init)) => return (State::Init(init), Err(err)),
            };

            (State::Connected(connected), Ok(()))
        })
    }

    fn try_accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        match self.state.read().as_ref() {
            State::Listen(listen) => listen.try_accept() as _,
            State::Init(_) | State::Connected(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is not listening")
            }
        }
    }
}

pub(super) const SHUT_READ_EVENTS: IoEvents =
    IoEvents::RDHUP.union(IoEvents::IN).union(IoEvents::HUP);
pub(super) const SHUT_WRITE_EVENTS: IoEvents = IoEvents::OUT.union(IoEvents::HUP);

impl Pollable for UnixStreamSocket {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.state.read().check_io_events())
    }
}

impl SocketPrivate for UnixStreamSocket {
    fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Relaxed)
    }

    fn set_nonblocking(&self, nonblocking: bool) {
        self.is_nonblocking.store(nonblocking, Ordering::Relaxed);
    }
}

impl Socket for UnixStreamSocket {
    fn bind(&self, socket_addr: SocketAddr) -> Result<()> {
        let addr = UnixSocketAddr::try_from(socket_addr)?;

        match self.state.write().as_mut() {
            State::Init(init) => init.bind(addr),
            State::Connected(connected) => connected.bind(addr),
            State::Listen(_) => {
                // Listening sockets are always already bound.
                addr.bind_unnamed()
            }
        }
    }

    fn connect(&self, socket_addr: SocketAddr) -> Result<()> {
        let remote_addr = UnixSocketAddr::try_from(socket_addr)?.connect()?;
        let backlog = get_backlog(&remote_addr)?;

        if self.is_nonblocking() {
            self.try_connect(&backlog)
        } else {
            backlog.pause_until(|| self.try_connect(&backlog))
        }
    }

    fn listen(&self, backlog: usize) -> Result<()> {
        const SOMAXCONN: usize = 4096;

        // Linux allows a maximum of `backlog + 1` sockets in the backlog queue. Although this
        // seems to be mostly an implementation detail, we follow the exact Linux behavior to
        // ensure that our regression tests pass with the Linux kernel.
        let backlog = backlog.saturating_add(1).min(SOMAXCONN);

        let mut state = self.state.write();

        state.borrow_result(|owned_state| {
            let init = match owned_state {
                State::Init(init) => init,
                State::Listen(listener) => {
                    listener.listen(backlog);
                    return (State::Listen(listener), Ok(()));
                }
                State::Connected(connected) => {
                    return (
                        State::Connected(connected),
                        Err(Error::with_message(
                            Errno::EINVAL,
                            "the socket is connected",
                        )),
                    );
                }
            };

            let listener = match init.listen(backlog, self.pollee.clone()) {
                Ok(listener) => listener,
                Err((err, init)) => {
                    return (State::Init(init), Err(err));
                }
            };

            (State::Listen(listener), Ok(()))
        })
    }

    fn accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        self.block_on(IoEvents::IN, || self.try_accept())
    }

    fn shutdown(&self, cmd: SockShutdownCmd) -> Result<()> {
        match self.state.read().as_ref() {
            State::Init(init) => init.shutdown(cmd, &self.pollee),
            State::Listen(listen) => listen.shutdown(cmd, &self.pollee),
            State::Connected(connected) => connected.shutdown(cmd),
        }

        Ok(())
    }

    fn addr(&self) -> Result<SocketAddr> {
        let addr = match self.state.read().as_ref() {
            State::Init(init) => init.addr().cloned(),
            State::Listen(listen) => Some(listen.addr().clone()),
            State::Connected(connected) => connected.addr(),
        };

        Ok(addr.into())
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        let peer_addr = match self.state.read().as_ref() {
            State::Connected(connected) => connected.peer_addr(),
            State::Init(_) | State::Listen(_) => {
                return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected")
            }
        };

        Ok(peer_addr.into())
    }

    fn sendmsg(
        &self,
        reader: &mut dyn MultiRead,
        message_header: MessageHeader,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        // TODO: Deal with flags
        if !flags.is_all_supported() {
            warn!("unsupported flags: {:?}", flags);
        }

        let MessageHeader {
            control_message, ..
        } = message_header;

        if control_message.is_some() {
            // TODO: Support sending control message
            warn!("sending control message is not supported");
        }

        self.block_on(IoEvents::OUT, || self.try_send(reader, flags))
    }

    fn recvmsg(
        &self,
        writer: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, MessageHeader)> {
        // TODO: Deal with flags
        if !flags.is_all_supported() {
            warn!("unsupported flags: {:?}", flags);
        }

        let received_bytes = self.block_on(IoEvents::IN, || self.try_recv(writer, flags))?;

        // TODO: Receive control message

        let message_header = MessageHeader::new(None, None);

        Ok((received_bytes, message_header))
    }
}
