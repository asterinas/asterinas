use crate::events::IoEvents;
use crate::fs::file_handle::FileLike;
use crate::fs::utils::StatusFlags;
use crate::net::socket::options::{
    Error, Linger, RecvBuf, ReuseAddr, ReusePort, SendBuf, SocketOption,
};
use crate::net::socket::util::options::{SocketOptionSet, MIN_RECVBUF, MIN_SENDBUF};
use crate::net::socket::util::{
    send_recv_flags::SendRecvFlags, shutdown_cmd::SockShutdownCmd, socket_addr::SocketAddr,
};
use crate::net::socket::Socket;
use crate::prelude::*;
use crate::process::signal::Poller;
use crate::{match_sock_option_mut, match_sock_option_ref};
use options::{Congestion, MaxSegment, WindowClamp};
use util::{TcpOptionSet, DEFAULT_MAXSEG};

use connected::ConnectedStream;
use connecting::ConnectingStream;
use init::InitStream;
use listen::ListenStream;
use options::NoDelay;
use smoltcp::wire::IpEndpoint;

mod connected;
mod connecting;
mod init;
mod listen;
pub mod options;
mod util;

pub use self::util::CongestionControl;

pub struct StreamSocket {
    options: RwLock<OptionSet>,
    state: RwLock<State>,
}

enum State {
    // Start state
    Init(Arc<InitStream>),
    // Intermediate state
    Connecting(Arc<ConnectingStream>),
    // Final State 1
    Connected(Arc<ConnectedStream>),
    // Final State 2
    Listen(Arc<ListenStream>),
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
    pub fn new(nonblocking: bool) -> Self {
        let options = OptionSet::new();
        let state = State::Init(InitStream::new(nonblocking));
        Self {
            options: RwLock::new(options),
            state: RwLock::new(state),
        }
    }

    fn is_nonblocking(&self) -> bool {
        match &*self.state.read() {
            State::Init(init) => init.is_nonblocking(),
            State::Connecting(connecting) => connecting.is_nonblocking(),
            State::Connected(connected) => connected.is_nonblocking(),
            State::Listen(listen) => listen.is_nonblocking(),
        }
    }

    fn set_nonblocking(&self, nonblocking: bool) {
        match &*self.state.read() {
            State::Init(init) => init.set_nonblocking(nonblocking),
            State::Connecting(connecting) => connecting.set_nonblocking(nonblocking),
            State::Connected(connected) => connected.set_nonblocking(nonblocking),
            State::Listen(listen) => listen.set_nonblocking(nonblocking),
        }
    }

    fn do_connect(&self, remote_endpoint: &IpEndpoint) -> Result<Arc<ConnectingStream>> {
        let mut state = self.state.write();
        let init_stream = match &*state {
            State::Init(init_stream) => init_stream,
            State::Listen(_) | State::Connecting(_) | State::Connected(_) => {
                return_errno_with_message!(Errno::EINVAL, "cannot connect")
            }
        };

        let connecting = init_stream.connect(remote_endpoint)?;
        *state = State::Connecting(connecting.clone());
        Ok(connecting)
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
        let state = self.state.read();
        match &*state {
            State::Init(init) => init.poll(mask, poller),
            State::Connecting(connecting) => connecting.poll(mask, poller),
            State::Connected(connected) => connected.poll(mask, poller),
            State::Listen(listen) => listen.poll(mask, poller),
        }
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
        let state = self.state.read();
        match &*state {
            State::Init(init_stream) => init_stream.bind(endpoint),
            _ => return_errno_with_message!(Errno::EINVAL, "cannot bind"),
        }
    }

    fn connect(&self, socket_addr: SocketAddr) -> Result<()> {
        let remote_endpoint = socket_addr.try_into()?;

        let connecting_stream = self.do_connect(&remote_endpoint)?;
        match connecting_stream.wait_conn() {
            Ok(connected_stream) => {
                *self.state.write() = State::Connected(connected_stream);
                Ok(())
            }
            Err((err, init_stream)) => {
                *self.state.write() = State::Init(init_stream);
                Err(err)
            }
        }
    }

    fn listen(&self, backlog: usize) -> Result<()> {
        let mut state = self.state.write();
        let init_stream = match &*state {
            State::Init(init_stream) => init_stream,
            State::Connecting(connecting_stream) => {
                return_errno_with_message!(Errno::EINVAL, "cannot listen for a connecting stream")
            }
            State::Listen(listen_stream) => {
                return_errno_with_message!(Errno::EINVAL, "cannot listen for a listening stream")
            }
            State::Connected(_) => return_errno_with_message!(Errno::EINVAL, "cannot listen"),
        };

        let listener = init_stream.listen(backlog)?;
        *state = State::Listen(listener);
        Ok(())
    }

    fn accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        let listen_stream = match &*self.state.read() {
            State::Listen(listen_stream) => listen_stream.clone(),
            _ => return_errno_with_message!(Errno::EINVAL, "the socket is not listening"),
        };

        let (connected_stream, remote_endpoint) = {
            let listen_stream = listen_stream.clone();
            listen_stream.accept()?
        };

        let accepted_socket = {
            let state = RwLock::new(State::Connected(connected_stream));
            Arc::new(StreamSocket {
                options: RwLock::new(OptionSet::new()),
                state,
            })
        };

        let socket_addr = remote_endpoint.try_into()?;
        Ok((accepted_socket, socket_addr))
    }

    fn shutdown(&self, cmd: SockShutdownCmd) -> Result<()> {
        let state = self.state.read();
        match &*state {
            State::Connected(connected_stream) => connected_stream.shutdown(cmd),
            // TDOD: shutdown listening stream
            _ => return_errno_with_message!(Errno::EINVAL, "cannot shutdown"),
        }
    }

    fn addr(&self) -> Result<SocketAddr> {
        let state = self.state.read();
        let local_endpoint = match &*state {
            State::Init(init_stream) => init_stream.local_endpoint(),
            State::Connecting(connecting_stream) => connecting_stream.local_endpoint(),
            State::Listen(listen_stream) => listen_stream.local_endpoint(),
            State::Connected(connected_stream) => connected_stream.local_endpoint(),
        }?;
        local_endpoint.try_into()
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        let state = self.state.read();
        let remote_endpoint = match &*state {
            State::Init(init_stream) => {
                return_errno_with_message!(Errno::EINVAL, "init socket does not have peer")
            }
            State::Connecting(connecting_stream) => connecting_stream.remote_endpoint(),
            State::Listen(listen_stream) => {
                return_errno_with_message!(Errno::EINVAL, "listening socket does not have peer")
            }
            State::Connected(connected_stream) => connected_stream.remote_endpoint(),
        }?;
        remote_endpoint.try_into()
    }

    fn recvfrom(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<(usize, SocketAddr)> {
        let connected_stream = match &*self.state.read() {
            State::Connected(connected_stream) => connected_stream.clone(),
            _ => return_errno_with_message!(Errno::EINVAL, "the socket is not connected"),
        };

        let (recv_size, remote_endpoint) = connected_stream.recvfrom(buf, flags)?;
        let socket_addr = remote_endpoint.try_into()?;
        Ok((recv_size, socket_addr))
    }

    fn sendto(
        &self,
        buf: &[u8],
        remote: Option<SocketAddr>,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        debug_assert!(remote.is_none());
        if remote.is_some() {
            return_errno_with_message!(Errno::EINVAL, "tcp socked should not provide remote addr");
        }

        let connected_stream = match &*self.state.read() {
            State::Connected(connected_stream) => connected_stream.clone(),
            _ => return_errno_with_message!(Errno::EINVAL, "the socket is not connected"),
        };
        connected_stream.sendto(buf, flags)
    }

    fn get_option(&self, option: &mut dyn SocketOption) -> Result<()> {
        let options = self.options.read();
        match_sock_option_mut!(option, {
            // Socket Options
            socket_errors: Error => {
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
                let maxseg = match &*self.state.read() {
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
                } else{
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
