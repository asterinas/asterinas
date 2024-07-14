// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use connected::ConnectedStream;
use connecting::ConnectingStream;
use init::InitStream;
use listen::ListenStream;
use options::{Congestion, MaxSegment, NoDelay, WindowClamp};
use smoltcp::wire::IpEndpoint;
use takeable::Takeable;
use util::{TcpOptionSet, DEFAULT_MAXSEG};

use super::UNSPECIFIED_LOCAL_ENDPOINT;
use crate::{
    events::{IoEvents, Observer},
    fs::{file_handle::FileLike, utils::StatusFlags},
    match_sock_option_mut, match_sock_option_ref,
    net::{
        poll_ifaces,
        socket::{
            options::{
                Error as SocketError, Linger, RecvBuf, ReuseAddr, ReusePort, SendBuf, SocketOption,
            },
            util::{
                copy_message_from_user, copy_message_to_user, create_message_buffer,
                options::{SocketOptionSet, MIN_RECVBUF, MIN_SENDBUF},
                send_recv_flags::SendRecvFlags,
                shutdown_cmd::SockShutdownCmd,
                socket_addr::SocketAddr,
                MessageHeader,
            },
            Socket,
        },
    },
    prelude::*,
    process::signal::{Pollable, Pollee, Poller},
    util::IoVec,
};

mod connected;
mod connecting;
mod init;
mod listen;
pub mod options;
mod util;

use self::connecting::NonConnectedStream;
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

    // Returns `None` to block the task and wait for the connection to be established, and returns
    // `Some(_)` if blocking is not necessary or not allowed.
    fn start_connect(&self, remote_endpoint: &IpEndpoint) -> Option<Result<()>> {
        let is_nonblocking = self.is_nonblocking();
        let mut state = self.state.write();

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

    fn finish_connect(&self) -> Result<()> {
        let mut state = self.state.write();

        state.borrow_result(|owned_state| {
            let State::Connecting(connecting_stream) = owned_state else {
                debug_assert!(false, "the socket unexpectedly left the connecting state");
                return (
                    owned_state,
                    Err(Error::with_message(
                        Errno::EINVAL,
                        "the socket is not connecting",
                    )),
                );
            };

            let connected_stream = match connecting_stream.into_result() {
                Ok(connected_stream) => connected_stream,
                Err((err, NonConnectedStream::Init(init_stream))) => {
                    init_stream.init_pollee(&self.pollee);
                    return (State::Init(init_stream), Err(err));
                }
                Err((err, NonConnectedStream::Connecting(connecting_stream))) => {
                    return (State::Connecting(connecting_stream), Err(err));
                }
            };
            connected_stream.init_pollee(&self.pollee);

            (State::Connected(connected_stream), Ok(()))
        })
    }

    fn check_connect(&self) -> Result<()> {
        // Hold the lock in advance to avoid deadlocks.
        let mut options = self.options.write();
        let mut state = self.state.write();

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
        let state = self.state.read();

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

    fn try_recv(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<(usize, SocketAddr)> {
        let state = self.state.read();

        let connected_stream = match state.as_ref() {
            State::Connected(connected_stream) => connected_stream,
            State::Init(_) | State::Listen(_) => {
                return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected")
            }
            State::Connecting(_) => {
                return_errno_with_message!(Errno::EAGAIN, "the socket is connecting")
            }
        };

        let received = connected_stream.try_recv(buf, flags).map(|recv_bytes| {
            connected_stream.update_io_events(&self.pollee);

            let remote_endpoint = connected_stream.remote_endpoint();
            (recv_bytes, remote_endpoint.into())
        });

        drop(state);
        poll_ifaces();

        received
    }

    fn recv(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<(usize, SocketAddr)> {
        if self.is_nonblocking() {
            self.try_recv(buf, flags)
        } else {
            self.wait_events(IoEvents::IN, || self.try_recv(buf, flags))
        }
    }

    fn try_send(&self, buf: &[u8], flags: SendRecvFlags) -> Result<usize> {
        let state = self.state.read();

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

        let sent_bytes = connected_stream.try_send(buf, flags).map(|sent_bytes| {
            connected_stream.update_io_events(&self.pollee);
            sent_bytes
        });

        drop(state);
        poll_ifaces();

        sent_bytes
    }

    fn send(&self, buf: &[u8], flags: SendRecvFlags) -> Result<usize> {
        if self.is_nonblocking() {
            self.try_send(buf, flags)
        } else {
            self.wait_events(IoEvents::OUT, || self.try_send(buf, flags))
        }
    }

    #[must_use]
    fn update_io_events(&self) -> bool {
        let state = self.state.read();
        match state.as_ref() {
            State::Init(_) => false,
            State::Connecting(connecting_stream) => connecting_stream.update_io_events(),
            State::Listen(listen_stream) => {
                listen_stream.update_io_events(&self.pollee);
                false
            }
            State::Connected(connected_stream) => {
                connected_stream.update_io_events(&self.pollee);
                false
            }
        }
    }
}

impl Pollable for StreamSocket {
    fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
    }
}

impl FileLike for StreamSocket {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        // TODO: Set correct flags
        let flags = SendRecvFlags::empty();
        self.recv(buf, flags).map(|(len, _)| len)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        // TODO: Set correct flags
        let flags = SendRecvFlags::empty();
        self.send(buf, flags)
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

        let mut state = self.state.write();

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

            let bound_socket = match init_stream.bind(&endpoint) {
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

        self.wait_events(IoEvents::OUT, || self.check_connect())
    }

    fn listen(&self, backlog: usize) -> Result<()> {
        let mut state = self.state.write();

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
            self.wait_events(IoEvents::IN, || self.try_accept())
        }
    }

    fn shutdown(&self, cmd: SockShutdownCmd) -> Result<()> {
        let state = self.state.read();
        match state.as_ref() {
            State::Connected(connected_stream) => connected_stream.shutdown(cmd),
            // TDOD: shutdown listening stream
            _ => return_errno_with_message!(Errno::EINVAL, "cannot shutdown"),
        }
    }

    fn addr(&self) -> Result<SocketAddr> {
        let state = self.state.read();
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
        let state = self.state.read();
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
        io_vecs: &[IoVec],
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

        let buf = copy_message_from_user(io_vecs);

        self.send(&buf, flags)
    }

    fn recvmsg(&self, io_vecs: &[IoVec], flags: SendRecvFlags) -> Result<(usize, MessageHeader)> {
        // TODO: Deal with flags
        debug_assert!(flags.is_all_supported());

        let mut buf = create_message_buffer(io_vecs);

        let (received_bytes, _) = self.recv(&mut buf, flags)?;

        let copied_bytes = {
            let message = &buf[..received_bytes];
            copy_message_to_user(io_vecs, message)
        };

        // TODO: Receive control message

        // According to <https://elixir.bootlin.com/linux/v6.0.9/source/net/ipv4/tcp.c#L2645>,
        // peer address is ignored for connected socket.
        let message_header = MessageHeader::new(None, None);

        Ok((copied_bytes, message_header))
    }

    fn get_option(&self, option: &mut dyn SocketOption) -> Result<()> {
        // Note that the socket error has to be handled separately, because it is automatically
        // cleared after reading.
        match_sock_option_mut!(option, {
            socket_errors: SocketError => {
                let mut options = self.options.write();
                let sock_errors = options.socket.sock_errors();
                socket_errors.set(sock_errors);
                options.socket.set_sock_errors(None);

                return Ok(());
            },
            _ => ()
        });

        let options = self.options.read();

        match_sock_option_mut!(option, {
            // Socket options:
            socket_reuse_addr: ReuseAddr => {
                let reuse_addr = options.socket.reuse_addr();
                socket_reuse_addr.set(reuse_addr);
            },
            socket_send_buf: SendBuf => {
                let send_buf = options.socket.send_buf();
                socket_send_buf.set(send_buf);
            },
            socket_recv_buf: RecvBuf => {
                let recv_buf = options.socket.recv_buf();
                socket_recv_buf.set(recv_buf);
            },
            socket_reuse_port: ReusePort => {
                let reuse_port = options.socket.reuse_port();
                socket_reuse_port.set(reuse_port);
            },
            // TCP options:
            tcp_no_delay: NoDelay => {
                let no_delay = options.tcp.no_delay();
                tcp_no_delay.set(no_delay);
            },
            tcp_congestion: Congestion => {
                let congestion = options.tcp.congestion();
                tcp_congestion.set(congestion);
            },
            tcp_maxseg: MaxSegment => {
                // It will always return the default MSS value defined above for an unconnected socket
                // and always return the actual current MSS for a connected one.

                // FIXME: how to get the current MSS?
                let maxseg = match self.state.read().as_ref() {
                    State::Init(_) | State::Listen(_) | State::Connecting(_) => DEFAULT_MAXSEG,
                    State::Connected(_) => options.tcp.maxseg(),
                };
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

        // FIXME: here we have only set the value of the option, without actually
        // making any real modifications.
        match_sock_option_ref!(option, {
            // Socket options:
            socket_recv_buf: RecvBuf => {
                let recv_buf = socket_recv_buf.get().unwrap();
                if *recv_buf <= MIN_RECVBUF {
                    options.socket.set_recv_buf(MIN_RECVBUF);
                } else {
                    options.socket.set_recv_buf(*recv_buf);
                }
            },
            socket_send_buf: SendBuf => {
                let send_buf = socket_send_buf.get().unwrap();
                if *send_buf <= MIN_SENDBUF {
                    options.socket.set_send_buf(MIN_SENDBUF);
                } else {
                    options.socket.set_send_buf(*send_buf);
                }
            },
            socket_reuse_addr: ReuseAddr => {
                let reuse_addr = socket_reuse_addr.get().unwrap();
                options.socket.set_reuse_addr(*reuse_addr);
            },
            socket_reuse_port: ReusePort => {
                let reuse_port = socket_reuse_port.get().unwrap();
                options.socket.set_reuse_port(*reuse_port);
            },
            socket_linger: Linger => {
                let linger = socket_linger.get().unwrap();
                options.socket.set_linger(*linger);
            },
            // TCP options:
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
                let half_recv_buf = (options.socket.recv_buf()) / 2;
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

impl Observer<()> for StreamSocket {
    fn on_events(&self, _events: &()) {
        let conn_ready = self.update_io_events();

        if conn_ready {
            // Hold the lock in advance to avoid race conditions.
            let mut options = self.options.write();

            let result = self.finish_connect();
            options.socket.set_sock_errors(result.err());
        }
    }
}
