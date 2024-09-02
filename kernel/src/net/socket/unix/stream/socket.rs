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
    fs::{
        file_handle::FileLike,
        utils::{InodeMode, Metadata, StatusFlags},
    },
    net::socket::{
        unix::UnixSocketAddr,
        util::{send_recv_flags::SendRecvFlags, socket_addr::SocketAddr, MessageHeader},
        SockShutdownCmd, Socket,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::{MultiRead, MultiWrite},
};

pub struct UnixStreamSocket {
    state: RwMutex<Takeable<State>>,
    is_nonblocking: AtomicBool,
}

impl UnixStreamSocket {
    pub(super) fn new_init(init: Init, is_nonblocking: bool) -> Arc<Self> {
        Arc::new(Self {
            state: RwMutex::new(Takeable::new(State::Init(init))),
            is_nonblocking: AtomicBool::new(is_nonblocking),
        })
    }

    pub(super) fn new_connected(connected: Connected, is_nonblocking: bool) -> Arc<Self> {
        Arc::new(Self {
            state: RwMutex::new(Takeable::new(State::Connected(connected))),
            is_nonblocking: AtomicBool::new(is_nonblocking),
        })
    }
}

enum State {
    Init(Init),
    Listen(Listener),
    Connected(Connected),
}

impl UnixStreamSocket {
    pub fn new(is_nonblocking: bool) -> Arc<Self> {
        Self::new_init(Init::new(), is_nonblocking)
    }

    pub fn new_pair(is_nonblocking: bool) -> (Arc<Self>, Arc<Self>) {
        let (conn_a, conn_b) = Connected::new_pair(None, None, None, None);
        (
            Self::new_connected(conn_a, is_nonblocking),
            Self::new_connected(conn_b, is_nonblocking),
        )
    }

    fn send(&self, reader: &mut dyn MultiRead, flags: SendRecvFlags) -> Result<usize> {
        if self.is_nonblocking() {
            self.try_send(reader, flags)
        } else {
            self.wait_events(IoEvents::OUT, None, || self.try_send(reader, flags))
        }
    }

    fn try_send(&self, buf: &mut dyn MultiRead, _flags: SendRecvFlags) -> Result<usize> {
        match self.state.read().as_ref() {
            State::Connected(connected) => connected.try_write(buf),
            State::Init(_) | State::Listen(_) => {
                return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected")
            }
        }
    }

    fn recv(&self, writer: &mut dyn MultiWrite, flags: SendRecvFlags) -> Result<usize> {
        if self.is_nonblocking() {
            self.try_recv(writer, flags)
        } else {
            self.wait_events(IoEvents::IN, None, || self.try_recv(writer, flags))
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

            let connected = match backlog.push_incoming(init) {
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

    fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Relaxed)
    }

    fn set_nonblocking(&self, nonblocking: bool) {
        self.is_nonblocking.store(nonblocking, Ordering::Relaxed);
    }
}

impl Pollable for UnixStreamSocket {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        let inner = self.state.read();
        match inner.as_ref() {
            State::Init(init) => init.poll(mask, poller),
            State::Listen(listen) => listen.poll(mask, poller),
            State::Connected(connected) => connected.poll(mask, poller),
        }
    }
}

impl FileLike for UnixStreamSocket {
    fn as_socket(self: Arc<Self>) -> Option<Arc<dyn Socket>> {
        Some(self)
    }

    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        // TODO: Set correct flags
        let flags = SendRecvFlags::empty();
        let read_len = self.recv(writer, flags)?;
        Ok(read_len)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        // TODO: Set correct flags
        let flags = SendRecvFlags::empty();
        self.send(reader, flags)
    }

    fn status_flags(&self) -> StatusFlags {
        if self.is_nonblocking() {
            StatusFlags::O_NONBLOCK
        } else {
            StatusFlags::empty()
        }
    }

    fn set_status_flags(&self, new_flags: StatusFlags) -> Result<()> {
        self.set_nonblocking(new_flags.contains(StatusFlags::O_NONBLOCK));
        Ok(())
    }

    fn metadata(&self) -> Metadata {
        // This is a dummy implementation.
        // TODO: Add "SockFS" and link `UnixStreamSocket` to it.
        Metadata::new_socket(
            0,
            InodeMode::from_bits_truncate(0o140777),
            aster_block::BLOCK_SIZE,
        )
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

            let listener = match init.listen(backlog) {
                Ok(listener) => listener,
                Err((err, init)) => {
                    return (State::Init(init), Err(err));
                }
            };

            (State::Listen(listener), Ok(()))
        })
    }

    fn accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        if self.is_nonblocking() {
            self.try_accept()
        } else {
            self.wait_events(IoEvents::IN, None, || self.try_accept())
        }
    }

    fn shutdown(&self, cmd: SockShutdownCmd) -> Result<()> {
        match self.state.read().as_ref() {
            State::Init(init) => init.shutdown(cmd),
            State::Listen(listen) => listen.shutdown(cmd),
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
        debug_assert!(flags.is_all_supported());

        let MessageHeader {
            control_message, ..
        } = message_header;

        if control_message.is_some() {
            // TODO: Support sending control message
            warn!("sending control message is not supported");
        }

        self.send(reader, flags)
    }

    fn recvmsg(
        &self,
        writer: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, MessageHeader)> {
        // TODO: Deal with flags
        debug_assert!(flags.is_all_supported());

        let received_bytes = self.recv(writer, flags)?;

        // TODO: Receive control message

        let message_header = MessageHeader::new(None, None);

        Ok((received_bytes, message_header))
    }
}
