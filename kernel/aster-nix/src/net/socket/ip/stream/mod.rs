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
                options::{SocketOptionSet, MIN_RECVBUF, MIN_SENDBUF},
                send_recv_flags::SendRecvFlags,
                shutdown_cmd::SockShutdownCmd,
                socket_addr::SocketAddr,
            },
            Socket,
        },
    },
    prelude::*,
    process::signal::{Pollee, Poller},
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

    fn start_connect(&self, remote_endpoint: &IpEndpoint) -> Result<()> {
        let mut state = self.state.write();

        state.borrow_result(|owned_state| {
            let State::Init(init_stream) = owned_state else {
                return (
                    owned_state,
                    Err(Error::with_message(Errno::EINVAL, "cannot connect")),
                );
            };

            let connecting_stream = match init_stream.connect(remote_endpoint) {
                Ok(connecting_stream) => connecting_stream,
                Err((err, init_stream)) => {
                    return (State::Init(init_stream), Err(err));
                }
            };
            connecting_stream.init_pollee(&self.pollee);

            (State::Connecting(connecting_stream), Ok(()))
        })
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

    fn try_accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        let state = self.state.read();

        let State::Listen(listen_stream) = state.as_ref() else {
            return_errno_with_message!(Errno::EINVAL, "the socket is not listening");
        };

        let connected_stream = listen_stream.try_accept()?;
        listen_stream.update_io_events(&self.pollee);

        let remote_endpoint = connected_stream.remote_endpoint();
        let accepted_socket = Self::new_connected(connected_stream);
        Ok((accepted_socket, remote_endpoint.try_into()?))
    }

    fn try_recvfrom(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<(usize, SocketAddr)> {
        let state = self.state.read();

        let State::Connected(connected_stream) = state.as_ref() else {
            return_errno_with_message!(Errno::EINVAL, "the socket is not connected");
        };
        let recv_bytes = connected_stream.try_recvfrom(buf, flags)?;
        connected_stream.update_io_events(&self.pollee);
        Ok((recv_bytes, connected_stream.remote_endpoint().try_into()?))
    }

    fn try_sendto(&self, buf: &[u8], flags: SendRecvFlags) -> Result<usize> {
        let state = self.state.read();

        let State::Connected(connected_stream) = state.as_ref() else {
            return_errno_with_message!(Errno::EINVAL, "the socket is not connected");
        };
        let sent_bytes = connected_stream.try_sendto(buf, flags)?;
        connected_stream.update_io_events(&self.pollee);
        Ok(sent_bytes)
    }

    // TODO: Support timeout
    fn wait_events<F, R>(&self, mask: IoEvents, mut cond: F) -> Result<R>
    where
        F: FnMut() -> Result<R>,
    {
        let poller = Poller::new();

        loop {
            match cond() {
                Err(err) if err.error() == Errno::EAGAIN => (),
                result => return result,
            };

            let events = self.poll(mask, Some(&poller));
            if !events.is_empty() {
                continue;
            }

            poller.wait()?;
        }
    }

    fn update_io_events(&self) {
        let state = self.state.read();
        match state.as_ref() {
            State::Init(_) => (),
            State::Connecting(connecting_stream) => {
                connecting_stream.update_io_events(&self.pollee)
            }
            State::Listen(listen_stream) => listen_stream.update_io_events(&self.pollee),
            State::Connected(connected_stream) => connected_stream.update_io_events(&self.pollee),
        }
    }
}

impl FileLike for StreamSocket {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        // FIXME: set correct flags
        let flags = SendRecvFlags::empty();
        let (recv_len, _) = self.recvfrom(buf, flags)?;
        Ok(recv_len)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        // FIXME: set correct flags
        let flags = SendRecvFlags::empty();
        self.sendto(buf, None, flags)
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
    }

    fn status_flags(&self) -> StatusFlags {
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
}

impl Socket for StreamSocket {
    fn bind(&self, socket_addr: SocketAddr) -> Result<()> {
        let endpoint = socket_addr.try_into()?;

        let mut state = self.state.write();

        state.borrow_result(|owned_state| {
            let State::Init(init_stream) = owned_state else {
                return (
                    owned_state,
                    Err(Error::with_message(Errno::EINVAL, "cannot bind")),
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

    // TODO: Support nonblocking mode
    fn connect(&self, socket_addr: SocketAddr) -> Result<()> {
        let remote_endpoint = socket_addr.try_into()?;
        self.start_connect(&remote_endpoint)?;

        poll_ifaces();
        self.wait_events(IoEvents::OUT, || self.finish_connect())
    }

    fn listen(&self, backlog: usize) -> Result<()> {
        let mut state = self.state.write();

        state.borrow_result(|owned_state| {
            let State::Init(init_stream) = owned_state else {
                return (
                    owned_state,
                    Err(Error::with_message(Errno::EINVAL, "cannot listen")),
                );
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
        poll_ifaces();
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
            State::Init(init_stream) => init_stream.local_endpoint()?,
            State::Connecting(connecting_stream) => connecting_stream.local_endpoint(),
            State::Listen(listen_stream) => listen_stream.local_endpoint(),
            State::Connected(connected_stream) => connected_stream.local_endpoint(),
        };
        local_endpoint.try_into()
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        let state = self.state.read();
        let remote_endpoint = match state.as_ref() {
            State::Init(init_stream) => {
                return_errno_with_message!(Errno::EINVAL, "init socket does not have peer")
            }
            State::Connecting(connecting_stream) => connecting_stream.remote_endpoint(),
            State::Listen(listen_stream) => {
                return_errno_with_message!(Errno::EINVAL, "listening socket does not have peer")
            }
            State::Connected(connected_stream) => connected_stream.remote_endpoint(),
        };
        remote_endpoint.try_into()
    }

    fn recvfrom(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<(usize, SocketAddr)> {
        debug_assert!(flags.is_all_supported());

        poll_ifaces();
        if self.is_nonblocking() {
            self.try_recvfrom(buf, flags)
        } else {
            self.wait_events(IoEvents::IN, || self.try_recvfrom(buf, flags))
        }
    }

    fn sendto(
        &self,
        buf: &[u8],
        remote: Option<SocketAddr>,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        debug_assert!(flags.is_all_supported());

        if remote.is_some() {
            return_errno_with_message!(Errno::EINVAL, "tcp socked should not provide remote addr");
        }

        let sent_bytes = if self.is_nonblocking() {
            self.try_sendto(buf, flags)?
        } else {
            self.wait_events(IoEvents::OUT, || self.try_sendto(buf, flags))?
        };
        poll_ifaces();
        Ok(sent_bytes)
    }

    fn get_option(&self, option: &mut dyn SocketOption) -> Result<()> {
        let options = self.options.read();
        match_sock_option_mut!(option, {
            // Socket Options
            socket_errors: SocketError => {
                let sock_errors = options.socket.sock_errors();
                socket_errors.set(sock_errors);
            },
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
            // Tcp Options
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
            _ => return_errno_with_message!(Errno::ENOPROTOOPT, "get unknown option")
        });
        Ok(())
    }

    fn set_option(&self, option: &dyn SocketOption) -> Result<()> {
        let mut options = self.options.write();
        // FIXME: here we have only set the value of the option, without actually
        // making any real modifications.
        match_sock_option_ref!(option, {
            // Socket options
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
            // Tcp options
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
                    return_errno_with_message!(Errno::EINVAL, "New maxseg should be in allowed range.");
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
            _ => return_errno_with_message!(Errno::ENOPROTOOPT, "set unknown option")
        });
        Ok(())
    }
}

impl Observer<()> for StreamSocket {
    fn on_events(&self, events: &()) {
        self.update_io_events();
    }
}
