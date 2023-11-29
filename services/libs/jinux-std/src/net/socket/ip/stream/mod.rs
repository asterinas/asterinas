use crate::events::IoEvents;
use crate::fs::{file_handle::FileLike, utils::StatusFlags};
use crate::net::iface::IpEndpoint;
use crate::net::socket::{
    util::{
        send_recv_flags::SendRecvFlags, shutdown_cmd::SockShutdownCmd,
        sock_options::SockOptionName, sockaddr::SocketAddr,
    },
    Socket,
};
use crate::prelude::*;
use crate::process::signal::Poller;

use self::{
    connected::ConnectedStream, connecting::ConnectingStream, init::InitStream,
    listen::ListenStream,
};

mod connected;
mod connecting;
mod init;
mod listen;

pub struct StreamSocket {
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

impl StreamSocket {
    pub fn new(nonblocking: bool) -> Self {
        let state = State::Init(Arc::new(InitStream::new(nonblocking)));
        Self {
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

        let connecting = Arc::new(init_stream.connect(remote_endpoint)?);
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

    fn as_socket(&self) -> Option<&dyn Socket> {
        Some(self)
    }
}

impl Socket for StreamSocket {
    fn bind(&self, sockaddr: SocketAddr) -> Result<()> {
        let endpoint = sockaddr.try_into()?;
        let state = self.state.read();
        match &*state {
            State::Init(init_stream) => init_stream.bind(endpoint),
            _ => return_errno_with_message!(Errno::EINVAL, "cannot bind"),
        }
    }

    fn connect(&self, sockaddr: SocketAddr) -> Result<()> {
        let remote_endpoint = sockaddr.try_into()?;

        let connecting_stream = self.do_connect(&remote_endpoint)?;
        match connecting_stream.wait_conn() {
            Ok(connected_stream) => {
                let connected_stream = Arc::new(connected_stream);
                *self.state.write() = State::Connected(connected_stream);
                Ok(())
            }
            Err((err, init_stream)) => {
                let init_stream = Arc::new(init_stream);
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

        let listener = Arc::new(init_stream.listen(backlog)?);
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
            let state = RwLock::new(State::Connected(Arc::new(connected_stream)));
            Arc::new(StreamSocket { state })
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

    fn sock_option(&self, optname: &SockOptionName) -> Result<&[u8]> {
        return_errno_with_message!(Errno::EINVAL, "getsockopt not implemented");
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
}
