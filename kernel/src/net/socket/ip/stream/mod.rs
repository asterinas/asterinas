// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use aster_bigtcp::{socket::SocketEventObserver, wire::IpEndpoint};
use connected::ConnectedStream;
use connecting::ConnectingStream;
use init::InitStream;
use listen::ListenStream;
use options::{Congestion, MaxSegment, NoDelay, WindowClamp};
use ostd::sync::{RwLockReadGuard, RwLockWriteGuard};
use takeable::Takeable;
use util::TcpOptionSet;

use super::UNSPECIFIED_LOCAL_ENDPOINT;
use crate::{
    events::{IoEvents, Observer},
    fs::{file_handle::FileLike, utils::StatusFlags},
    match_sock_option_mut, match_sock_option_ref,
    net::{
        iface::poll_ifaces,
        socket::{
            options::{Error as SocketError, SocketOption},
            util::{
                options::SocketOptionSet, send_recv_flags::SendRecvFlags,
                shutdown_cmd::SockShutdownCmd, socket_addr::SocketAddr, MessageHeader,
            },
            Socket,
        },
    },
    prelude::*,
    process::signal::{Pollable, Pollee, Poller},
    util::{MultiRead, MultiWrite},
};

mod connected;
mod connecting;
mod init;
mod listen;
pub mod options;
mod util;

pub use self::util::CongestionControl;

pub struct StreamSocket {
    options: RwLock<OptionSet>,
    state: RwLock<Takeable<State>>,
    is_nonblocking: AtomicBool,
    pollee: Pollee,
}

enum State {
    // Start state
    Init(InitStream),
    // Intermediate state
    Connecting(ConnectingStream),
    // Final State 1
    Connected(ConnectedStream),
    // Final State 2
    Listen(ListenStream),
}

#[derive(Debug, Clone)]
struct OptionSet {
    socket: SocketOptionSet,
    tcp: TcpOptionSet,
}

impl OptionSet {
    fn new() -> Self {
        let socket = SocketOptionSet::new_tcp();
        let tcp = TcpOptionSet::new();
        OptionSet { socket, tcp }
    }
}

impl StreamSocket {
    pub fn new(nonblocking: bool) -> Arc<Self> {
        Arc::new_cyclic(|me| {
            let init_stream = InitStream::new(me.clone() as _);
            let pollee = Pollee::new(IoEvents::empty());
            init_stream.init_pollee(&pollee);
            Self {
                options: RwLock::new(OptionSet::new()),
                state: RwLock::new(Takeable::new(State::Init(init_stream))),
                is_nonblocking: AtomicBool::new(nonblocking),
                pollee,
            }
        })
    }

    fn new_connected(connected_stream: ConnectedStream) -> Arc<Self> {
        Arc::new_cyclic(move |me| {
            let pollee = Pollee::new(IoEvents::empty());
            connected_stream.set_observer(me.clone() as _);
            connected_stream.init_pollee(&pollee);
            Self {
                options: RwLock::new(OptionSet::new()),
                state: RwLock::new(Takeable::new(State::Connected(connected_stream))),
                is_nonblocking: AtomicBool::new(false),
                pollee,
            }
        })
    }

    fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Relaxed)
    }

    fn set_nonblocking(&self, nonblocking: bool) {
        self.is_nonblocking.store(nonblocking, Ordering::Relaxed);
    }

    /// Ensures that the socket state is up to date and obtains a read lock on it.
    ///
    /// For a description of what "up-to-date" means, see [`Self::update_connecting`].
    fn read_updated_state(&self) -> RwLockReadGuard<Takeable<State>> {
        loop {
            let state = self.state.read();
            match state.as_ref() {
                State::Connecting(connecting_stream) if connecting_stream.has_result() => (),
                _ => return state,
            };
            drop(state);

            self.update_connecting();
        }
    }

    /// Ensures that the socket state is up to date and obtains a write lock on it.
    ///
    /// For a description of what "up-to-date" means, see [`Self::update_connecting`].
    fn write_updated_state(&self) -> RwLockWriteGuard<Takeable<State>> {
        self.update_connecting().1
    }

    /// Updates the socket state if the socket is an obsolete connecting socket.
    ///
    /// A connecting socket can become obsolete because some network events can set the socket to
    /// connected state (if the connection succeeds) or initial state (if the connection is
    /// refused) in [`Self::update_io_events`], but the state transition is delayed until the user
    /// operates on the socket to avoid too many locks in the interrupt handler.
    ///
    /// This method performs the delayed state transition to ensure that the state is up to date
    /// and returns the guards of the write-locked options and state.
    fn update_connecting(
        &self,
    ) -> (
        RwLockWriteGuard<OptionSet>,
        RwLockWriteGuard<Takeable<State>>,
    ) {
        // Hold the lock in advance to avoid race conditions.
        let mut options = self.options.write();
        let mut state = self.state.write_irq_disabled();

        match state.as_ref() {
            State::Connecting(connection_stream) if connection_stream.has_result() => (),
            _ => return (options, state),
        }

        let result = state.borrow_result(|owned_state| {
            let State::Connecting(connecting_stream) = owned_state else {
                unreachable!("`State::Connecting` is checked before calling `borrow_result`");
            };

            let connected_stream = match connecting_stream.into_result() {
                Ok(connected_stream) => connected_stream,
                Err((err, init_stream)) => {
                    init_stream.init_pollee(&self.pollee);
                    return (State::Init(init_stream), Err(err));
                }
            };
            connected_stream.init_pollee(&self.pollee);

            (State::Connected(connected_stream), Ok(()))
        });
        options.socket.set_sock_errors(result.err());

        (options, state)
    }

    // Returns `None` to block the task and wait for the connection to be established, and returns
    // `Some(_)` if blocking is not necessary or not allowed.
    fn start_connect(&self, remote_endpoint: &IpEndpoint) -> Option<Result<()>> {
        let is_nonblocking = self.is_nonblocking();
        let mut state = self.write_updated_state();

        let result_or_block = state.borrow_result(|mut owned_state| {
            let init_stream = match owned_state {
                State::Init(init_stream) => init_stream,
                State::Connecting(_) if is_nonblocking => {
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
                State::Connected(ref mut connected_stream) => {
                    let err = connected_stream.check_new();
                    return (owned_state, Some(err));
                }
                State::Listen(_) => {
                    return (
                        owned_state,
                        Some(Err(Error::with_message(
                            Errno::EISCONN,
                            "the socket is listening",
                        ))),
                    );
                }
            };

            let connecting_stream = match init_stream.connect(remote_endpoint) {
                Ok(connecting_stream) => connecting_stream,
                Err((err, init_stream)) => {
                    return (State::Init(init_stream), Some(Err(err)));
                }
            };
            connecting_stream.init_pollee(&self.pollee);

            (
                State::Connecting(connecting_stream),
                if is_nonblocking {
                    Some(Err(Error::with_message(
                        Errno::EINPROGRESS,
                        "the socket is connecting",
                    )))
                } else {
                    None
                },
            )
        });

        drop(state);
        poll_ifaces();

        result_or_block
    }

    fn check_connect(&self) -> Result<()> {
        let (mut options, mut state) = self.update_connecting();

        match state.as_mut() {
            State::Connecting(_) => {
                return_errno_with_message!(Errno::EAGAIN, "the connection is pending")
            }
            State::Connected(connected_stream) => connected_stream.check_new(),
            State::Init(_) | State::Listen(_) => {
                let sock_errors = options.socket.sock_errors();
                options.socket.set_sock_errors(None);
                sock_errors.map(Err).unwrap_or(Ok(()))
            }
        }
    }

    fn try_accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        let state = self.read_updated_state();

        let State::Listen(listen_stream) = state.as_ref() else {
            return_errno_with_message!(Errno::EINVAL, "the socket is not listening");
        };

        let accepted = listen_stream.try_accept().map(|connected_stream| {
            listen_stream.update_io_events(&self.pollee);

            let remote_endpoint = connected_stream.remote_endpoint();
            let accepted_socket = Self::new_connected(connected_stream);
            (accepted_socket as _, remote_endpoint.into())
        });

        drop(state);
        poll_ifaces();

        accepted
    }

    fn try_recv(
        &self,
        writer: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, SocketAddr)> {
        let state = self.read_updated_state();

        let connected_stream = match state.as_ref() {
            State::Connected(connected_stream) => connected_stream,
            State::Init(_) | State::Listen(_) => {
                return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected")
            }
            State::Connecting(_) => {
                return_errno_with_message!(Errno::EAGAIN, "the socket is connecting")
            }
        };

        let received = connected_stream.try_recv(writer, flags).map(|recv_bytes| {
            let remote_endpoint = connected_stream.remote_endpoint();
            (recv_bytes, remote_endpoint.into())
        });

        drop(state);
        poll_ifaces();

        received
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

    fn try_send(&self, reader: &mut dyn MultiRead, flags: SendRecvFlags) -> Result<usize> {
        let state = self.read_updated_state();

        let connected_stream = match state.as_ref() {
            State::Connected(connected_stream) => connected_stream,
            State::Init(_) | State::Listen(_) => {
                // TODO: Trigger `SIGPIPE` if `MSG_NOSIGNAL` is not specified
                return_errno_with_message!(Errno::EPIPE, "the socket is not connected");
            }
            State::Connecting(_) => {
                // FIXME: Linux indeed allows data to be buffered at this point. Can we do
                // something similar?
                return_errno_with_message!(Errno::EAGAIN, "the socket is connecting")
            }
        };

        let sent_bytes = connected_stream.try_send(reader, flags);

        drop(state);
        poll_ifaces();

        sent_bytes
    }

    fn send(&self, reader: &mut dyn MultiRead, flags: SendRecvFlags) -> Result<usize> {
        if self.is_nonblocking() {
            self.try_send(reader, flags)
        } else {
            self.wait_events(IoEvents::OUT, None, || self.try_send(reader, flags))
        }
    }

    fn update_io_events(&self) {
        let state = self.state.read();
        match state.as_ref() {
            State::Init(_) => (),
            State::Connecting(connecting_stream) => {
                connecting_stream.update_io_events(&self.pollee)
            }
            State::Listen(listen_stream) => {
                listen_stream.update_io_events(&self.pollee);
            }
            State::Connected(connected_stream) => {
                connected_stream.update_io_events(&self.pollee);
            }
        }

        // Note: Network events can cause a state transition from `State::Connecting` to
        // `State::Connected`/`State::Init`. The state transition is delayed until
        // `update_connecting`is triggered by user events, see that method for details.
    }
}

impl Pollable for StreamSocket {
    fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
    }
}

impl FileLike for StreamSocket {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        // TODO: Set correct flags
        let flags = SendRecvFlags::empty();
        self.recv(writer, flags).map(|(len, _)| len)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        // TODO: Set correct flags
        let flags = SendRecvFlags::empty();
        self.send(reader, flags)
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

    fn as_socket(self: Arc<Self>) -> Option<Arc<dyn Socket>> {
        Some(self)
    }

    fn register_observer(
        &self,
        observer: Weak<dyn Observer<IoEvents>>,
        mask: IoEvents,
    ) -> Result<()> {
        self.pollee.register_observer(observer, mask);
        Ok(())
    }

    fn unregister_observer(
        &self,
        observer: &Weak<dyn Observer<IoEvents>>,
    ) -> Option<Weak<dyn Observer<IoEvents>>> {
        self.pollee.unregister_observer(observer)
    }
}

impl Socket for StreamSocket {
    fn bind(&self, socket_addr: SocketAddr) -> Result<()> {
        let endpoint = socket_addr.try_into()?;

        let can_reuse = self.options.read().socket.reuse_addr();
        let mut state = self.write_updated_state();

        state.borrow_result(|owned_state| {
            let State::Init(init_stream) = owned_state else {
                return (
                    owned_state,
                    Err(Error::with_message(
                        Errno::EINVAL,
                        "the socket is already bound to an address",
                    )),
                );
            };

            let bound_socket = match init_stream.bind(&endpoint, can_reuse) {
                Ok(bound_socket) => bound_socket,
                Err((err, init_stream)) => {
                    return (State::Init(init_stream), Err(err));
                }
            };

            (State::Init(InitStream::new_bound(bound_socket)), Ok(()))
        })
    }

    fn connect(&self, socket_addr: SocketAddr) -> Result<()> {
        let remote_endpoint = socket_addr.try_into()?;

        if let Some(result) = self.start_connect(&remote_endpoint) {
            return result;
        }

        self.wait_events(IoEvents::OUT, None, || self.check_connect())
    }

    fn listen(&self, backlog: usize) -> Result<()> {
        let mut state = self.write_updated_state();

        state.borrow_result(|owned_state| {
            let init_stream = match owned_state {
                State::Init(init_stream) => init_stream,
                State::Listen(listen_stream) => {
                    return (State::Listen(listen_stream), Ok(()));
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

            let listen_stream = match init_stream.listen(backlog) {
                Ok(listen_stream) => listen_stream,
                Err((err, init_stream)) => {
                    return (State::Init(init_stream), Err(err));
                }
            };
            listen_stream.init_pollee(&self.pollee);

            (State::Listen(listen_stream), Ok(()))
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
        let state = self.read_updated_state();
        match state.as_ref() {
            State::Connected(connected_stream) => connected_stream.shutdown(cmd),
            // TODO: shutdown listening stream
            _ => return_errno_with_message!(Errno::EINVAL, "cannot shutdown"),
        }
    }

    fn addr(&self) -> Result<SocketAddr> {
        let state = self.read_updated_state();
        let local_endpoint = match state.as_ref() {
            State::Init(init_stream) => init_stream
                .local_endpoint()
                .unwrap_or(UNSPECIFIED_LOCAL_ENDPOINT),
            State::Connecting(connecting_stream) => connecting_stream.local_endpoint(),
            State::Listen(listen_stream) => listen_stream.local_endpoint(),
            State::Connected(connected_stream) => connected_stream.local_endpoint(),
        };
        Ok(local_endpoint.into())
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        let state = self.read_updated_state();
        let remote_endpoint = match state.as_ref() {
            State::Init(_) | State::Listen(_) => {
                return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected")
            }
            State::Connecting(connecting_stream) => connecting_stream.remote_endpoint(),
            State::Connected(connected_stream) => connected_stream.remote_endpoint(),
        };
        Ok(remote_endpoint.into())
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

        // According to the Linux man pages, `EISCONN` _may_ be returned when the destination
        // address is specified for a connection-mode socket. In practice, the destination address
        // is simply ignored. We follow the same behavior as the Linux implementation to ignore it.

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

        let (received_bytes, _) = self.recv(writer, flags)?;

        // TODO: Receive control message

        // According to <https://elixir.bootlin.com/linux/v6.0.9/source/net/ipv4/tcp.c#L2645>,
        // peer address is ignored for connected socket.
        let message_header = MessageHeader::new(None, None);

        Ok((received_bytes, message_header))
    }

    fn get_option(&self, option: &mut dyn SocketOption) -> Result<()> {
        match_sock_option_mut!(option, {
            socket_errors: SocketError => {
                let mut options = self.update_connecting().0;
                options.socket.get_and_clear_sock_errors(socket_errors);
                return Ok(());
            },
            _ => ()
        });

        let options = self.options.read();

        match options.socket.get_option(option) {
            Err(err) if err.error() == Errno::ENOPROTOOPT => (),
            res => return res,
        }

        // FIXME: Here we only return the previously set values, without actually
        // asking the underlying sockets for the real, effective values.
        match_sock_option_mut!(option, {
            tcp_no_delay: NoDelay => {
                let no_delay = options.tcp.no_delay();
                tcp_no_delay.set(no_delay);
            },
            tcp_congestion: Congestion => {
                let congestion = options.tcp.congestion();
                tcp_congestion.set(congestion);
            },
            tcp_maxseg: MaxSegment => {
                let maxseg = options.tcp.maxseg();
                tcp_maxseg.set(maxseg);
            },
            tcp_window_clamp: WindowClamp => {
                let window_clamp = options.tcp.window_clamp();
                tcp_window_clamp.set(window_clamp);
            },
            _ => return_errno_with_message!(Errno::ENOPROTOOPT, "the socket option to get is unknown")
        });

        Ok(())
    }

    fn set_option(&self, option: &dyn SocketOption) -> Result<()> {
        let mut options = self.options.write();

        match options.socket.set_option(option) {
            Err(err) if err.error() == Errno::ENOPROTOOPT => (),
            res => return res,
        }

        // FIXME: Here we have only set the value of the option, without actually
        // making any real modifications.
        match_sock_option_ref!(option, {
            tcp_no_delay: NoDelay => {
                let no_delay = tcp_no_delay.get().unwrap();
                options.tcp.set_no_delay(*no_delay);
            },
            tcp_congestion: Congestion => {
                let congestion = tcp_congestion.get().unwrap();
                options.tcp.set_congestion(*congestion);
            },
            tcp_maxseg: MaxSegment => {
                const MIN_MAXSEG: u32 = 536;
                const MAX_MAXSEG: u32 = 65535;

                let maxseg = tcp_maxseg.get().unwrap();
                if *maxseg < MIN_MAXSEG || *maxseg > MAX_MAXSEG {
                    return_errno_with_message!(Errno::EINVAL, "the maximum segment size is out of bounds");
                }
                options.tcp.set_maxseg(*maxseg);
            },
            tcp_window_clamp: WindowClamp => {
                let window_clamp = tcp_window_clamp.get().unwrap();
                let half_recv_buf = options.socket.recv_buf() / 2;
                if *window_clamp <= half_recv_buf {
                    options.tcp.set_window_clamp(half_recv_buf);
                } else {
                    options.tcp.set_window_clamp(*window_clamp);
                }
            },
            _ => return_errno_with_message!(Errno::ENOPROTOOPT, "the socket option to be set is unknown")
        });

        Ok(())
    }
}

impl SocketEventObserver for StreamSocket {
    fn on_events(&self) {
        self.update_io_events();
    }
}

impl Drop for StreamSocket {
    fn drop(&mut self) {
        self.state.write().take();

        poll_ifaces();
    }
}
