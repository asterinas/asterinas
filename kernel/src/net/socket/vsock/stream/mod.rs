// SPDX-License-Identifier: MPL-2.0

mod connected;
mod connecting;
mod init;
mod listen;

use core::sync::atomic::{AtomicBool, Ordering};

use connected::ConnectedStream;
use connecting::{ConnResult, ConnectingStream};
use init::InitStream;
use listen::ListenStream;
use takeable::Takeable;

use crate::{
    events::IoEvents,
    fs::{file::FileLike, pseudofs::SockFs, vfs::path::Path},
    net::socket::{
        Socket,
        options::{Error as SocketError, SocketOption, macros::sock_option_mut},
        private::SocketPrivate,
        util::{MessageHeader, SendRecvFlags, SockShutdownCmd, SocketAddr},
        vsock::addr::{UNSPECIFIED_VSOCK_ADDR, VsockSocketAddr},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    util::{MultiRead, MultiWrite},
};

pub struct VsockStreamSocket {
    state: Mutex<Takeable<State>>,
    is_nonblocking: AtomicBool,
    // Note that for vsock, all pollee notifications and invalidations live in the transport module
    // (e.g., `super::transport`) rather than in this module.
    pollee: Pollee,
    pseudo_path: Path,
}

enum State {
    Init(InitStream),
    Connecting(ConnectingStream),
    Connected(ConnectedStream),
    Listen(ListenStream),
}

impl VsockStreamSocket {
    pub fn new(is_nonblocking: bool) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            state: Mutex::new(Takeable::new(State::Init(InitStream::new()))),
            is_nonblocking: AtomicBool::new(is_nonblocking),
            pollee: Pollee::new(),
            pseudo_path: SockFs::new_path(),
        }))
    }

    fn start_connect(&self, remote_addr: VsockSocketAddr) -> Option<Result<()>> {
        let mut state = self.lock_updated_state();

        state.borrow_result(|owned_state| {
            let init_stream = match owned_state {
                State::Init(init_stream) => init_stream,
                State::Connecting(_) if self.is_nonblocking() => {
                    return (
                        owned_state,
                        Some(Err(Error::with_message(
                            Errno::EALREADY,
                            "the socket is connecting",
                        ))),
                    );
                }
                State::Connecting(_) => {
                    return (owned_state, None);
                }
                State::Connected(mut connected_stream) => {
                    let result = connected_stream.finish_last_connect();
                    return (State::Connected(connected_stream), Some(result));
                }
                State::Listen(_) => {
                    return (
                        owned_state,
                        Some(Err(Error::with_message(
                            Errno::EINVAL,
                            "the socket is listening",
                        ))),
                    );
                }
            };

            if !init_stream.is_connect_done() {
                return (
                    State::Init(init_stream),
                    Some(Err(Error::with_message(
                        Errno::EALREADY,
                        "a previous connection attempt exists",
                    ))),
                );
            }

            match init_stream.connect(remote_addr, &self.pollee) {
                Ok(connecting_stream) if self.is_nonblocking() => (
                    State::Connecting(connecting_stream),
                    Some(Err(Error::with_message(
                        Errno::EINPROGRESS,
                        "the socket is connecting",
                    ))),
                ),
                Ok(connecting_stream) => (State::Connecting(connecting_stream), None),
                Err((error, init_stream)) => (State::Init(init_stream), Some(Err(error))),
            }
        })

        // The pollee should have already been invalidated in the transport module.
    }

    fn check_connect(&self) -> Result<()> {
        let mut state = self.lock_updated_state();

        match state.as_mut() {
            State::Init(init_stream) => {
                if let Some(error) = init_stream.test_and_clear_error(&self.pollee) {
                    return Err(error);
                }
                return_errno_with_message!(
                    Errno::ECONNABORTED,
                    "the error code for the connection failure is not available"
                );
            }
            State::Connecting(_) => {
                return_errno_with_message!(Errno::EAGAIN, "the socket is connecting")
            }
            State::Connected(connected_stream) => connected_stream.finish_last_connect(),
            State::Listen(_) => {
                return_errno_with_message!(Errno::EISCONN, "the socket is listening")
            }
        }
    }

    fn try_accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        let state = self.lock_updated_state();
        let State::Listen(listen_stream) = state.as_ref() else {
            return_errno_with_message!(Errno::EINVAL, "the socket is not listening");
        };

        let connected = listen_stream.try_accept()?;

        let peer_addr = connected.remote_addr().into();
        let pollee = connected.pollee().clone();

        let accepted = Arc::new(Self {
            state: Mutex::new(Takeable::new(State::Connected(connected))),
            is_nonblocking: AtomicBool::new(false),
            pollee,
            pseudo_path: SockFs::new_path(),
        });

        Ok((accepted, peer_addr))
    }

    fn try_send(&self, reader: &mut dyn MultiRead, flags: SendRecvFlags) -> Result<usize> {
        let mut state = self.lock_updated_state();
        let State::Connected(connected_stream) = state.as_mut() else {
            return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected");
        };

        connected_stream.try_send(reader, flags)
    }

    fn try_recv(&self, writer: &mut dyn MultiWrite, flags: SendRecvFlags) -> Result<usize> {
        let mut state = self.lock_updated_state();
        let State::Connected(connected_stream) = state.as_mut() else {
            return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected");
        };

        connected_stream.try_recv(writer, flags)
    }

    fn test_and_clear_error(&self) -> Option<Error> {
        let mut state = self.lock_updated_state();

        match state.as_mut() {
            State::Init(init_stream) => init_stream.test_and_clear_error(&self.pollee),
            State::Connecting(_) => None,
            State::Connected(connected_stream) => connected_stream.test_and_clear_error(),
            State::Listen(_) => None,
        }
    }

    fn check_io_events(&self) -> IoEvents {
        let state = self.lock_updated_state();

        match state.as_ref() {
            State::Init(init_stream) => init_stream.check_io_events(),
            State::Connecting(connecting_stream) => connecting_stream.check_io_events(),
            State::Connected(connected_stream) => connected_stream.check_io_events(),
            State::Listen(listen_stream) => listen_stream.check_io_events(),
        }
    }

    /// Locks the state and updates it if needed.
    ///
    /// If the current state is [`State::Connecting`] and the connect attempt has finished, this
    /// method transitions the state to the latest one before returning the lock guard.
    ///
    /// Callers should always use this method instead of locking the state directly.
    fn lock_updated_state(&self) -> MutexGuard<'_, Takeable<State>> {
        let mut state = self.state.lock();

        let State::Connecting(connecting_stream) = state.as_ref() else {
            return state;
        };
        if !connecting_stream.has_result() {
            return state;
        }

        state.borrow(|owned_state| {
            let State::Connecting(connection_stream) = owned_state else {
                unreachable!();
            };
            match connection_stream.into_result() {
                ConnResult::Connecting(connection_stream) => State::Connecting(connection_stream),
                ConnResult::Connected(connected_stream) => State::Connected(connected_stream),
                ConnResult::Failed(init_stream) => State::Init(init_stream),
            }
        });

        state
    }
}

impl Pollable for VsockStreamSocket {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }
}

impl SocketPrivate for VsockStreamSocket {
    fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Relaxed)
    }

    fn set_nonblocking(&self, nonblocking: bool) {
        self.is_nonblocking.store(nonblocking, Ordering::Relaxed);
    }
}

impl Socket for VsockStreamSocket {
    fn bind(&self, socket_addr: SocketAddr) -> Result<()> {
        let addr = VsockSocketAddr::try_from(socket_addr)?;

        let mut state = self.lock_updated_state();
        let State::Init(init_stream) = state.as_mut() else {
            return_errno_with_message!(Errno::EINVAL, "the socket is already bound to an address");
        };

        init_stream.bind(addr)
    }

    fn connect(&self, socket_addr: SocketAddr) -> Result<()> {
        let remote_addr = VsockSocketAddr::try_from(socket_addr)?;

        if let Some(result) = self.start_connect(remote_addr) {
            return result;
        }

        // FIXME: Linux cancels pending packets and aborts the connection if a blocking `connect()`
        // is interrupted by a signal. Here, we follow the behavior of IP sockets by continuing to
        // establish the connection in the background.
        self.wait_events(IoEvents::OUT, None, || self.check_connect())
    }

    fn listen(&self, backlog: usize) -> Result<()> {
        let mut state = self.lock_updated_state();

        state.borrow_result(|owned_state| {
            let init_stream = match owned_state {
                State::Init(init_stream) => init_stream,
                State::Listen(ref listen_stream) => {
                    listen_stream.set_backlog(backlog);
                    return (owned_state, Ok(()));
                }
                State::Connecting(_) | State::Connected(_) => {
                    return (
                        owned_state,
                        Err(Error::with_message(
                            Errno::EINVAL,
                            "the socket is already connected",
                        )),
                    );
                }
            };

            match init_stream.listen(backlog, &self.pollee) {
                Ok(listen_stream) => (State::Listen(listen_stream), Ok(())),
                Err((error, init_stream)) => (State::Init(init_stream), Err(error)),
            }
        })

        // The pollee should have already been invalidated in the transport module.
    }

    fn accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        self.block_on(IoEvents::IN, || self.try_accept())
    }

    fn shutdown(&self, cmd: SockShutdownCmd) -> Result<()> {
        let mut state = self.lock_updated_state();

        match state.as_mut() {
            State::Init(init_stream) => init_stream.shutdown(cmd),
            State::Connecting(_) => {
                return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected")
            }
            State::Connected(connected_stream) => connected_stream.shutdown(cmd),
            State::Listen(_) => {
                return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected")
            }
        }
    }

    fn addr(&self) -> Result<SocketAddr> {
        let state = self.lock_updated_state();

        let local_addr = match state.as_ref() {
            State::Init(init_stream) => init_stream.local_addr(),
            State::Connecting(connecting_stream) => Some(connecting_stream.local_addr()),
            State::Connected(connected_stream) => Some(connected_stream.local_addr()),
            State::Listen(listen_stream) => Some(listen_stream.local_addr()),
        };

        Ok(local_addr.unwrap_or(UNSPECIFIED_VSOCK_ADDR).into())
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        let state = self.lock_updated_state();
        let State::Connected(connected_stream) = state.as_ref() else {
            return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected");
        };

        Ok(connected_stream.remote_addr().into())
    }

    // TODO: Support setting socket options

    fn get_option(&self, option: &mut dyn SocketOption) -> Result<()> {
        sock_option_mut!(match option {
            socket_errors @ SocketError => {
                socket_errors.set(self.test_and_clear_error());
                return Ok(());
            }
            _ => {}
        });

        // TODO: Support getting other socket options
        return_errno_with_message!(Errno::EOPNOTSUPP, "the socket option to be get is unknown");
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
            control_messages,
            addr,
        } = message_header;

        // According to the Linux man pages, `EISCONN` _may_ be returned when the destination
        // address is specified for a connection-mode socket. In practice, `sendmsg` on vosck
        // stream sockets will fail due to that. We follow the same behavior as the Linux
        // implementation.
        if addr.is_some() {
            let state = self.lock_updated_state();
            match state.as_ref() {
                State::Init(_) | State::Listen(_) | State::Connecting(_) => {
                    return_errno_with_message!(
                        Errno::EOPNOTSUPP,
                        "sending to a specific address is not allowed on vsock stream sockets"
                    );
                }
                State::Connected(_) => {
                    return_errno_with_message!(
                        Errno::EISCONN,
                        "sending to a specific address is not allowed on vsock stream sockets"
                    );
                }
            }
        }

        if !control_messages.is_empty() {
            // TODO: Support sending control message
            warn!("sending control message is not supported");
        }

        self.block_on(IoEvents::OUT, || self.try_send(reader, flags))

        // TODO: Trigger `SIGPIPE` if the error code is `EPIPE` and `MSG_NOSIGNAL` is not specified
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
        let message_header = MessageHeader::new(None, Vec::new());

        Ok((received_bytes, message_header))
    }

    fn pseudo_path(&self) -> &Path {
        &self.pseudo_path
    }
}
