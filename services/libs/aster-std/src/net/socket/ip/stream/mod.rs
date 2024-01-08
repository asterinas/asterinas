use core::mem;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::events::{IoEvents, Observer};
use crate::fs::{file_handle::FileLike, utils::StatusFlags};
use crate::net::iface::IpEndpoint;
use crate::net::poll_ifaces;
use crate::net::socket::{
    util::{
        send_recv_flags::SendRecvFlags, shutdown_cmd::SockShutdownCmd,
        sock_options::SockOptionName, sockaddr::SocketAddr,
    },
    Socket,
};
use crate::prelude::*;
use crate::process::signal::{Pollee, Poller};

use self::connecting::NonConnectedStream;
use self::{
    connected::ConnectedStream, connecting::ConnectingStream, init::InitStream,
    listen::ListenStream,
};

use super::DUMMY_LOCAL_ENDPOINT;

mod connected;
mod connecting;
mod init;
mod listen;

pub struct StreamSocket {
    state: RwLock<State>,
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
    // Poisoned state
    Poisoned,
}

impl StreamSocket {
    pub fn new(nonblocking: bool) -> Arc<Self> {
        Arc::new_cyclic(|me| {
            let init_stream = InitStream::new(me.clone() as _);
            let pollee = Pollee::new(IoEvents::empty());
            Self {
                state: RwLock::new(State::Init(init_stream)),
                is_nonblocking: AtomicBool::new(nonblocking),
                pollee,
            }
        })
    }

    fn new_connected(nonblocking: bool, connected_stream: ConnectedStream) -> Arc<Self> {
        Arc::new_cyclic(move |me| {
            let pollee = Pollee::new(IoEvents::empty());
            connected_stream.set_observer(me.clone() as _);
            connected_stream.reset_io_events(&pollee);
            Self {
                state: RwLock::new(State::Connected(connected_stream)),
                is_nonblocking: AtomicBool::new(nonblocking),
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

        let owned_state = mem::replace(&mut *state, State::Poisoned);
        let State::Init(init_stream) = owned_state else {
            *state = owned_state;
            return_errno_with_message!(Errno::EINVAL, "cannot connect")
        };

        let connecting_stream = match init_stream.connect(remote_endpoint) {
            Ok(connecting_stream) => connecting_stream,
            Err((err, init_stream)) => {
                *state = State::Init(init_stream);
                return Err(err);
            }
        };
        connecting_stream.reset_io_events(&self.pollee);
        *state = State::Connecting(connecting_stream);

        Ok(())
    }

    fn finish_connect(&self) -> Result<()> {
        let mut state = self.state.write();

        let owned_state = mem::replace(&mut *state, State::Poisoned);
        let State::Connecting(connecting_stream) = owned_state else {
            *state = owned_state;
            debug_assert!(false, "the socket unexpectedly left the connecting state");
            return_errno_with_message!(Errno::EINVAL, "the socket is not connecting");
        };

        let connected_stream = match connecting_stream.into_result() {
            Ok(connected_stream) => connected_stream,
            Err((err, NonConnectedStream::Init(init_stream))) => {
                *state = State::Init(init_stream);
                return Err(err);
            }
            Err((err, NonConnectedStream::Connecting(connecting_stream))) => {
                *state = State::Connecting(connecting_stream);
                return Err(err);
            }
        };
        connected_stream.reset_io_events(&self.pollee);
        *state = State::Connected(connected_stream);

        Ok(())
    }

    fn try_accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        let state = self.state.read();

        let State::Listen(listen_stream) = &*state else {
            return_errno_with_message!(Errno::EINVAL, "the socket is not listening");
        };

        let connected_stream = listen_stream.try_accept()?;
        listen_stream.update_io_events(&self.pollee);

        let remote_endpoint = connected_stream.remote_endpoint();
        let accepted_socket = Self::new_connected(self.is_nonblocking(), connected_stream);
        Ok((accepted_socket, remote_endpoint.into()))
    }

    fn try_recvfrom(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<(usize, SocketAddr)> {
        let state = self.state.read();

        let connected_stream = match &*state {
            State::Connected(connected_stream) => connected_stream,
            State::Init(_) | State::Listen(_) => {
                return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected")
            }
            State::Connecting(_) => {
                return_errno_with_message!(Errno::EAGAIN, "the socket is connecting")
            }
            State::Poisoned => {
                // FIXME: This error code has no Linux equivalent
                return_errno_with_message!(Errno::EINVAL, "the socket is poisoned");
            }
        };

        let recv_bytes = connected_stream.try_recvfrom(buf, flags)?;
        connected_stream.update_io_events(&self.pollee);
        Ok((recv_bytes, connected_stream.remote_endpoint().into()))
    }

    fn try_sendto(&self, buf: &[u8], flags: SendRecvFlags) -> Result<usize> {
        let state = self.state.read();

        let connected_stream = match &*state {
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
            State::Poisoned => {
                // FIXME: This error code has no Linux equivalent
                return_errno_with_message!(Errno::EINVAL, "the socket is poisoned");
            }
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
        match &*state {
            State::Init(_) | State::Poisoned => (),
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

    fn as_socket(&self) -> Option<&dyn Socket> {
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
    ) -> Result<Weak<dyn Observer<IoEvents>>> {
        self.pollee
            .unregister_observer(observer)
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "observer is not registered"))
    }
}

impl Socket for StreamSocket {
    fn bind(&self, sockaddr: SocketAddr) -> Result<()> {
        let endpoint = sockaddr.try_into()?;

        let mut state = self.state.write();

        let owned_state = mem::replace(&mut *state, State::Poisoned);
        let State::Init(init_stream) = owned_state else {
            *state = owned_state;
            // FIXME: The error code may not be correct if the socket is poisoned
            return_errno_with_message!(Errno::EINVAL, "the socket is already bound to an address");
        };

        let bound_socket = match init_stream.bind(&endpoint) {
            Ok(bound_socket) => bound_socket,
            Err((err, init_stream)) => {
                *state = State::Init(init_stream);
                return Err(err);
            }
        };
        *state = State::Init(InitStream::new_bound(bound_socket));

        Ok(())
    }

    // TODO: Support nonblocking mode
    fn connect(&self, sockaddr: SocketAddr) -> Result<()> {
        let remote_endpoint = sockaddr.try_into()?;
        self.start_connect(&remote_endpoint)?;

        poll_ifaces();
        self.wait_events(IoEvents::OUT, || self.finish_connect())
    }

    fn listen(&self, backlog: usize) -> Result<()> {
        let mut state = self.state.write();

        let owned_state = mem::replace(&mut *state, State::Poisoned);
        let init_stream = match owned_state {
            State::Init(init_stream) => init_stream,
            State::Listen(listen_stream) => {
                *state = State::Listen(listen_stream);
                return Ok(());
            }
            State::Connecting(_) | State::Connected(_) | State::Poisoned => {
                *state = owned_state;
                // FIXME: The error code may not be correct if the socket is poisoned
                return_errno_with_message!(Errno::EINVAL, "the socket is already connected");
            }
        };

        let listen_stream = match init_stream.listen(backlog) {
            Ok(listen_stream) => listen_stream,
            Err((err, init_stream)) => {
                *state = State::Init(init_stream);
                return Err(err);
            }
        };
        listen_stream.reset_io_events(&self.pollee);
        *state = State::Listen(listen_stream);

        Ok(())
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
        match &*state {
            State::Connected(connected_stream) => connected_stream.shutdown(cmd),
            // TDOD: shutdown listening stream
            _ => return_errno_with_message!(Errno::EINVAL, "cannot shutdown"),
        }
    }

    fn addr(&self) -> Result<SocketAddr> {
        let state = self.state.read();
        let local_endpoint = match &*state {
            State::Init(init_stream) => {
                init_stream.local_endpoint().unwrap_or(DUMMY_LOCAL_ENDPOINT)
            }
            State::Connecting(connecting_stream) => connecting_stream.local_endpoint(),
            State::Listen(listen_stream) => listen_stream.local_endpoint(),
            State::Connected(connected_stream) => connected_stream.local_endpoint(),
            State::Poisoned => {
                // FIXME: This error code has no Linux equivalent
                return_errno_with_message!(Errno::EINVAL, "the socket is poisoned");
            }
        };
        Ok(local_endpoint.into())
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        let state = self.state.read();
        let remote_endpoint = match &*state {
            State::Connecting(connecting_stream) => connecting_stream.remote_endpoint(),
            State::Connected(connected_stream) => connected_stream.remote_endpoint(),
            State::Init(_) | State::Listen(_) => {
                return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected")
            }
            State::Poisoned => {
                // FIXME: This error code has no Linux equivalent
                return_errno_with_message!(Errno::EINVAL, "the socket is poisoned");
            }
        };
        Ok(remote_endpoint.into())
    }

    fn sock_option(&self, optname: &SockOptionName) -> Result<&[u8]> {
        return_errno_with_message!(Errno::EINVAL, "getsockopt not implemented");
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

        // According to the Linux man pages, `EISCONN` *may* be returned when the destination
        // address is specified for a connection-mode socket. In practice, the destination address
        // is simply ignored. We follow the same behavior as the Linux implementation to ignore it.

        let sent_bytes = if self.is_nonblocking() {
            self.try_sendto(buf, flags)?
        } else {
            self.wait_events(IoEvents::OUT, || self.try_sendto(buf, flags))?
        };
        poll_ifaces();
        Ok(sent_bytes)
    }
}

impl Observer<()> for StreamSocket {
    fn on_events(&self, events: &()) {
        self.update_io_events();
    }
}
