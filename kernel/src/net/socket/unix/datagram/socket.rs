// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use aster_rights::ReadDupOp;

use super::message::{MessageQueue, MessageReceiver};
use crate::{
    events::IoEvents,
    fs::{path::Path, pseudofs::SockFs},
    net::socket::{
        Socket,
        options::{Error as SocketError, PeerCred, SocketOption, macros::sock_option_mut},
        private::SocketPrivate,
        unix::{CUserCred, UnixSocketAddr, cred::SocketCred, ctrl_msg::AuxiliaryData},
        util::{
            MessageHeader, SendRecvFlags, SockShutdownCmd, SocketAddr,
            options::{GetSocketLevelOption, SetSocketLevelOption, SocketOptionSet},
        },
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::{MultiRead, MultiWrite},
};

pub struct UnixDatagramSocket {
    local_receiver: MessageReceiver,
    remote_queue: RwLock<Option<Arc<MessageQueue>>>,
    options: RwLock<OptionSet>,
    // Since datagram sockets are not connection-oriented, they typically lack well-defined peer
    // credentials. According to the Linux implementation, however, peer credentials are recorded
    // when a socket pair is created using the `socketpair` system call.
    peer_cred: Option<SocketCred>,

    is_nonblocking: AtomicBool,
    is_write_shutdown: AtomicBool,
    pseudo_path: Path,
}

#[derive(Clone, Debug)]
struct OptionSet {
    socket: SocketOptionSet,
}

impl OptionSet {
    pub(self) fn new() -> Self {
        Self {
            socket: SocketOptionSet::new_unix_datagram(),
        }
    }
}

impl UnixDatagramSocket {
    pub fn new(is_nonblocking: bool) -> Arc<Self> {
        Arc::new(Self::new_raw(is_nonblocking))
    }

    pub fn new_pair(is_nonblocking: bool) -> (Arc<Self>, Arc<Self>) {
        let mut socket_a = Self::new_raw(is_nonblocking);
        let mut socket_b = Self::new_raw(is_nonblocking);

        let cred = SocketCred::<ReadDupOp>::new_current();
        socket_a.peer_cred = Some(cred.dup().restrict());
        socket_b.peer_cred = Some(cred.restrict());

        let remote_queue_a = socket_a.remote_queue.get_mut();
        let remote_queue_b = socket_b.remote_queue.get_mut();

        *remote_queue_a = Some(socket_b.local_receiver.queue().clone());
        *remote_queue_b = Some(socket_a.local_receiver.queue().clone());

        (Arc::new(socket_a), Arc::new(socket_b))
    }

    fn new_raw(is_nonblocking: bool) -> Self {
        Self {
            local_receiver: MessageReceiver::new(),
            remote_queue: RwLock::new(None),
            options: RwLock::new(OptionSet::new()),
            peer_cred: None,
            is_nonblocking: AtomicBool::new(is_nonblocking),
            is_write_shutdown: AtomicBool::new(false),
            pseudo_path: SockFs::new_path(),
        }
    }

    fn do_send(
        &self,
        reader: &mut dyn MultiRead,
        mut aux_data: AuxiliaryData,
        remote: Option<UnixSocketAddr>,
        _flags: SendRecvFlags,
    ) -> Result<usize> {
        if self.is_write_shutdown.load(Ordering::Relaxed) {
            return_errno_with_message!(Errno::EPIPE, "the socket is shut down for writing");
        }

        let queue = if let Some(remote_addr) = remote.as_ref() {
            let connected_addr = remote_addr.connect()?;
            MessageQueue::lookup_bound(&connected_addr)?
        } else {
            let remote_queue = self.remote_queue.read();
            remote_queue.clone().ok_or_else(|| {
                Error::with_message(Errno::ENOTCONN, "the socket is not connected")
            })?
        };

        let res = if self.is_nonblocking() {
            queue.try_send(reader, &mut aux_data, &self.local_receiver)
        } else {
            queue.block_send(|| queue.try_send(reader, &mut aux_data, &self.local_receiver))
        };

        // A connected socket will automatically be disconnected if the remote has been closed.
        if remote.is_none() && res.is_err_and(|err| err.error() == Errno::ECONNREFUSED) {
            let mut remote_queue = self.remote_queue.write();
            // Check to ensure that we are still connected to the same remote.
            if remote_queue
                .as_ref()
                .is_some_and(|remote| Arc::ptr_eq(remote, &queue))
            {
                *remote_queue = None;
            }
        }

        res
    }

    fn check_io_events(&self) -> IoEvents {
        // POLLOUT should be reported as long as there is space in the socket's send buffer.
        // Currently, we only limit the size of the receive buffer, not the send buffer. Therefore,
        // POLLOUT is always reported.
        let mut io_events = IoEvents::OUT;

        io_events |= self.local_receiver.check_io_events();

        if self.is_write_shutdown.load(Ordering::Relaxed) && io_events.contains(IoEvents::RDHUP) {
            io_events |= IoEvents::HUP;
        }

        io_events
    }
}

impl Pollable for UnixDatagramSocket {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.local_receiver
            .pollee()
            .poll_with(mask, poller, || self.check_io_events())
    }
}

impl SocketPrivate for UnixDatagramSocket {
    fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Relaxed)
    }

    fn set_nonblocking(&self, nonblocking: bool) {
        self.is_nonblocking.store(nonblocking, Ordering::Relaxed);
    }
}

impl Socket for UnixDatagramSocket {
    fn bind(&self, socket_addr: SocketAddr) -> Result<()> {
        let addr = UnixSocketAddr::try_from(socket_addr)?;
        self.local_receiver.bind(addr)
    }

    fn connect(&self, socket_addr: SocketAddr) -> Result<()> {
        let remote_addr = UnixSocketAddr::try_from(socket_addr)?;

        let connected_addr = remote_addr.connect()?;
        let queue = MessageQueue::lookup_bound(&connected_addr)?;

        let mut remote_queue = self.remote_queue.write();
        *remote_queue = Some(queue);

        Ok(())
    }

    fn shutdown(&self, cmd: SockShutdownCmd) -> Result<()> {
        let mut io_events = IoEvents::empty();

        if cmd.shut_read() {
            self.local_receiver.shutdown();
            io_events |= IoEvents::IN | IoEvents::RDHUP | IoEvents::HUP;
        }

        if cmd.shut_write() {
            self.is_write_shutdown.store(true, Ordering::Relaxed);
            io_events |= IoEvents::HUP;
        }

        self.local_receiver.pollee().notify(io_events);

        Ok(())
    }

    fn addr(&self) -> Result<SocketAddr> {
        Ok(self.local_receiver.addr().into())
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        let remote_queue = self.remote_queue.read();
        match remote_queue.as_ref() {
            Some(queue) => Ok(queue.addr().into()),
            None => return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected"),
        }
    }

    fn get_option(&self, option: &mut dyn SocketOption) -> Result<()> {
        sock_option_mut!(match option {
            socket_errors @ SocketError => {
                // TODO: Support socket errors for UNIX sockets
                socket_errors.set(None);
                return Ok(());
            }
            _ => (),
        });

        // Deal with UNIX-socket-specific socket-level options
        match do_unix_getsockopt(option, self) {
            Err(err) if err.error() == Errno::ENOPROTOOPT => (),
            res => return res,
        }

        let options = self.options.read();

        // Deal with socket-level options
        match options.socket.get_option(option, &self.local_receiver) {
            Err(err) if err.error() == Errno::ENOPROTOOPT => (),
            res => return res,
        }

        // TODO: Deal with socket options from other levels
        warn!("only socket-level options are supported");

        return_errno_with_message!(Errno::ENOPROTOOPT, "the socket option to get is unknown")
    }

    fn set_option(&self, option: &dyn SocketOption) -> Result<()> {
        let mut options = self.options.write();

        match options.socket.set_option(option, &self.local_receiver) {
            Err(err) if err.error() == Errno::ENOPROTOOPT => {
                // TODO: Deal with socket options from other levels
                warn!("only socket-level options are supported");
                return_errno_with_message!(
                    Errno::ENOPROTOOPT,
                    "the socket option to get is unknown"
                )
            }
            res => res.map(|_need_iface_poll| ()),
        }
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
            addr,
            control_messages,
        } = message_header;

        let remote_addr = match addr {
            Some(addr) => Some(addr.try_into()?),
            None => None,
        };

        let auxiliary_data = AuxiliaryData::from_control(control_messages)?;

        self.do_send(reader, auxiliary_data, remote_addr, flags)
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

        let (received_bytes, control_messages, peer_addr) =
            self.block_on(IoEvents::IN, || self.local_receiver.try_recv(writer))?;

        let message_header = MessageHeader::new(Some(peer_addr.into()), control_messages);

        Ok((received_bytes, message_header))
    }

    fn pseudo_path(&self) -> &Path {
        &self.pseudo_path
    }
}

fn do_unix_getsockopt(option: &mut dyn SocketOption, socket: &UnixDatagramSocket) -> Result<()> {
    sock_option_mut!(match option {
        socket_peer_cred @ PeerCred => {
            let peer_cred = socket
                .peer_cred
                .as_ref()
                .map(SocketCred::to_effective_c_cred)
                .unwrap_or_else(CUserCred::new_invalid);
            socket_peer_cred.set(peer_cred);
        }
        _ => return_errno_with_message!(
            Errno::ENOPROTOOPT,
            "the socket option to get is not UNIX-socket-specific"
        ),
    });

    Ok(())
}

impl GetSocketLevelOption for MessageReceiver {
    fn is_listening(&self) -> bool {
        false
    }
}

impl SetSocketLevelOption for MessageReceiver {
    fn set_pass_cred(&self, pass_cred: bool) {
        // TODO: According to the Linux man pages, "When this option is set and the socket
        // is not yet connected, a unique name in the abstract namespace will be generated
        // automatically." See <https://man7.org/linux/man-pages/man7/unix.7.html> for
        // details.

        self.set_pass_cred(pass_cred);
    }
}
