// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use aster_rights::ReadDupOp;
use takeable::Takeable;

use super::{
    connected::Connected,
    init::Init,
    listener::{get_backlog, Backlog, Listener},
};
use crate::{
    events::IoEvents,
    fs::{file_handle::FileLike, utils::EndpointState},
    match_sock_option_mut,
    net::socket::{
        options::{PeerCred, PeerGroups, SocketOption},
        private::SocketPrivate,
        unix::{cred::SocketCred, ctrl_msg::AuxiliaryData, CUserCred, UnixSocketAddr},
        util::{
            options::{GetSocketLevelOption, SetSocketLevelOption, SocketOptionSet},
            ControlMessage, MessageHeader, SendRecvFlags, SockShutdownCmd, SocketAddr,
        },
        Socket,
    },
    prelude::*,
    process::{
        signal::{PollHandle, Pollable, Pollee},
        Gid,
    },
    util::{MultiRead, MultiWrite},
};

pub struct UnixStreamSocket {
    // Lock order: `state` first, `options` second
    state: RwMutex<Takeable<State>>,
    options: RwMutex<OptionSet>,

    pollee: Pollee,
    is_nonblocking: AtomicBool,

    is_seqpacket: bool,
}

impl UnixStreamSocket {
    pub(super) fn new_init(init: Init, is_nonblocking: bool, is_seqpacket: bool) -> Arc<Self> {
        Arc::new(Self {
            state: RwMutex::new(Takeable::new(State::Init(init))),
            options: RwMutex::new(OptionSet::new()),
            pollee: Pollee::new(),
            is_nonblocking: AtomicBool::new(is_nonblocking),
            is_seqpacket,
        })
    }

    pub(super) fn new_connected(
        connected: Connected,
        options: OptionSet,
        is_nonblocking: bool,
        is_seqpacket: bool,
    ) -> Arc<Self> {
        let cloned_pollee = connected.cloned_pollee();
        Arc::new(Self {
            state: RwMutex::new(Takeable::new(State::Connected(connected))),
            options: RwMutex::new(options),
            pollee: cloned_pollee,
            is_nonblocking: AtomicBool::new(is_nonblocking),
            is_seqpacket,
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

    pub(self) fn peer_cred(&self) -> Option<CUserCred> {
        match self {
            Self::Init(_) => None,
            Self::Listen(listener) => Some(listener.cred().to_effective_c_cred()),
            Self::Connected(connected) => Some(connected.peer_cred().to_effective_c_cred()),
        }
    }

    pub(self) fn peer_groups(&self) -> Result<Arc<[Gid]>> {
        match self {
            State::Init(_) => {
                return_errno_with_message!(Errno::ENODATA, "the socket does not have peer groups")
            }
            State::Listen(listener) => Ok(listener.cred().groups()),
            State::Connected(connected) => Ok(connected.peer_cred().groups()),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct OptionSet {
    socket: SocketOptionSet,
}

impl OptionSet {
    pub(super) fn new() -> Self {
        Self {
            socket: SocketOptionSet::new_unix_stream(),
        }
    }
}

impl UnixStreamSocket {
    pub fn new(is_nonblocking: bool, is_seqpacket: bool) -> Arc<Self> {
        Self::new_init(Init::new(), is_nonblocking, is_seqpacket)
    }

    pub fn new_pair(is_nonblocking: bool, is_seqpacket: bool) -> (Arc<Self>, Arc<Self>) {
        let cred = SocketCred::<ReadDupOp>::new_current();
        let options = OptionSet::new();

        let (conn_a, conn_b) = Connected::new_pair(
            None,
            None,
            EndpointState::default(),
            EndpointState::default(),
            cred.dup().restrict(),
            cred.restrict(),
            &options.socket,
        );
        (
            Self::new_connected(conn_a, options, is_nonblocking, is_seqpacket),
            Self::new_connected(conn_b, OptionSet::new(), is_nonblocking, is_seqpacket),
        )
    }

    fn try_send(
        &self,
        buf: &mut dyn MultiRead,
        aux_data: &mut AuxiliaryData,
        _flags: SendRecvFlags,
    ) -> Result<usize> {
        match self.state.read().as_ref() {
            State::Connected(connected) => connected.try_write(buf, aux_data, self.is_seqpacket),
            State::Init(_) | State::Listen(_) => {
                return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected")
            }
        }
    }

    fn try_recv(
        &self,
        buf: &mut dyn MultiWrite,
        _flags: SendRecvFlags,
    ) -> Result<(usize, Vec<ControlMessage>)> {
        match self.state.read().as_ref() {
            State::Connected(connected) => connected.try_read(buf, self.is_seqpacket),
            State::Init(_) | State::Listen(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is not connected")
            }
        }
    }

    fn try_connect(&self, backlog: &Arc<Backlog>) -> Result<()> {
        let mut state = self.state.write();
        let options = self.options.read();

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

            let connected = match backlog.push_incoming(
                init,
                self.pollee.clone(),
                &options.socket,
                self.is_seqpacket,
            ) {
                Ok(connected) => connected,
                Err((err, init)) => return (State::Init(init), Err(err)),
            };

            (State::Connected(connected), Ok(()))
        })
    }

    fn try_accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        match self.state.read().as_ref() {
            State::Listen(listen) => listen.try_accept(self.is_seqpacket) as _,
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

            let listener = match init.listen(backlog, self.pollee.clone(), self.is_seqpacket) {
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
            control_messages,
            addr,
        } = message_header;

        // According to the Linux man pages, `EISCONN` _may_ be returned when the destination
        // address is specified for a connection-mode socket. In practice, `sendmsg` on UNIX stream
        // sockets will fail due to that. We follow the same behavior as the Linux implementation.
        if !self.is_seqpacket && addr.is_some() {
            match self.state.read().as_ref() {
                State::Init(_) | State::Listen(_) => return_errno_with_message!(
                    Errno::EOPNOTSUPP,
                    "sending to a specific address is not allowed on UNIX stream sockets"
                ),
                State::Connected(_) => return_errno_with_message!(
                    Errno::EISCONN,
                    "sending to a specific address is not allowed on UNIX stream sockets"
                ),
            }
        }
        let mut auxiliary_data = AuxiliaryData::from_control(control_messages)?;

        self.block_on(IoEvents::OUT, || {
            self.try_send(reader, &mut auxiliary_data, flags)
        })
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

        let (received_bytes, control_messages) =
            self.block_on(IoEvents::IN, || self.try_recv(writer, flags))?;

        let message_header = MessageHeader::new(None, control_messages);

        Ok((received_bytes, message_header))
    }

    fn get_option(&self, option: &mut dyn SocketOption) -> Result<()> {
        let state = self.state.read();
        let options = self.options.read();

        // Deal with UNIX-socket-specific socket-level options
        match do_unix_getsockopt(option, state.as_ref()) {
            Err(err) if err.error() == Errno::ENOPROTOOPT => (),
            res => return res,
        }

        // Deal with socket-level options
        match options.socket.get_option(option, state.as_ref()) {
            Err(err) if err.error() == Errno::ENOPROTOOPT => (),
            res => return res,
        }

        // TODO: Deal with socket options from other levels
        warn!("only socket-level options are supported");

        return_errno_with_message!(Errno::ENOPROTOOPT, "the socket option to get is unknown")
    }

    fn set_option(&self, option: &dyn SocketOption) -> Result<()> {
        let mut state = self.state.write();
        let mut options = self.options.write();

        match options.socket.set_option(option, state.as_mut()) {
            Ok(_) => Ok(()),
            Err(err) if err.error() == Errno::ENOPROTOOPT => {
                // TODO: Deal with socket options from other levels
                warn!("only socket-level options are supported");
                return_errno_with_message!(
                    Errno::ENOPROTOOPT,
                    "the socket option to get is unknown"
                )
            }
            Err(e) => Err(e),
        }
    }
}

fn do_unix_getsockopt(option: &mut dyn SocketOption, state: &State) -> Result<()> {
    match_sock_option_mut!(option, {
        socket_peer_cred: PeerCred => {
            let peer_cred = state.peer_cred().unwrap_or_else(CUserCred::new_invalid);
            socket_peer_cred.set(peer_cred);
        },
        socket_peer_groups: PeerGroups => {
            let groups = state.peer_groups()?;
            socket_peer_groups.set(groups);
        },
        _ => return_errno_with_message!(
            Errno::ENOPROTOOPT,
            "the socket option to get is not UNIX-socket-specific"
        )
    });

    Ok(())
}

impl GetSocketLevelOption for State {
    fn is_listening(&self) -> bool {
        matches!(self, Self::Listen(_))
    }
}

impl SetSocketLevelOption for State {
    fn set_pass_cred(&self, pass_cred: bool) {
        match self {
            Self::Init(_) => {
                // TODO: According to the Linux man pages, "When this option is set and the socket
                // is not yet connected, a unique name in the abstract namespace will be generated
                // automatically." See <https://man7.org/linux/man-pages/man7/unix.7.html> for
                // details.
            }
            Self::Listen(_) => {}
            Self::Connected(connected) => connected.set_pass_cred(pass_cred),
        }
    }
}
