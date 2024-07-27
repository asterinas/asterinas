// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::AtomicBool;

use atomic::Ordering;

use super::{
    connected::Connected,
    init::Init,
    listener::{push_incoming, Listener},
};
use crate::{
    events::{IoEvents, Observer},
    fs::{file_handle::FileLike, utils::StatusFlags},
    net::socket::{
        unix::UnixSocketAddr,
        util::{
            copy_message_from_user, copy_message_to_user, create_message_buffer,
            send_recv_flags::SendRecvFlags, socket_addr::SocketAddr, MessageHeader,
        },
        SockShutdownCmd, Socket,
    },
    prelude::*,
    process::signal::{Pollable, Poller},
    thread::Thread,
    util::IoVec,
};

pub struct UnixStreamSocket {
    state: RwLock<State>,
    is_nonblocking: AtomicBool,
}

impl UnixStreamSocket {
    pub(super) fn new_init(init: Init, is_nonblocking: bool) -> Arc<Self> {
        Arc::new(Self {
            state: RwLock::new(State::Init(init)),
            is_nonblocking: AtomicBool::new(is_nonblocking),
        })
    }

    pub(super) fn new_connected(connected: Connected, is_nonblocking: bool) -> Arc<Self> {
        Arc::new(Self {
            state: RwLock::new(State::Connected(connected)),
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
        let (conn_a, conn_b) = Connected::new_pair(None, None);
        (
            Self::new_connected(conn_a, is_nonblocking),
            Self::new_connected(conn_b, is_nonblocking),
        )
    }

    fn send(&self, buf: &[u8], flags: SendRecvFlags) -> Result<usize> {
        if self.is_nonblocking() {
            self.try_send(buf, flags)
        } else {
            self.wait_events(IoEvents::OUT, || self.try_send(buf, flags))
        }
    }

    fn try_send(&self, buf: &[u8], _flags: SendRecvFlags) -> Result<usize> {
        match &*self.state.read() {
            State::Connected(connected) => connected.try_write(buf),
            _ => return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected"),
        }
    }

    fn recv(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<usize> {
        if self.is_nonblocking() {
            self.try_recv(buf, flags)
        } else {
            self.wait_events(IoEvents::IN, || self.try_recv(buf, flags))
        }
    }

    fn try_recv(&self, buf: &mut [u8], _flags: SendRecvFlags) -> Result<usize> {
        match &*self.state.read() {
            State::Connected(connected) => connected.try_read(buf),
            _ => return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected"),
        }
    }

    fn try_accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        match &*self.state.read() {
            State::Listen(listen) => listen.try_accept() as _,
            _ => return_errno_with_message!(Errno::EINVAL, "the socket is not listening"),
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
    fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents {
        let inner = self.state.read();
        match &*inner {
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
        let mut buf = vec![0u8; writer.avail()];
        // TODO: Set correct flags
        let flags = SendRecvFlags::empty();
        let read_len = self.recv(&mut buf, flags)?;
        writer.write_fallible(&mut buf.as_slice().into())?;
        Ok(read_len)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        let buf = reader.collect()?;
        // TODO: Set correct flags
        let flags = SendRecvFlags::empty();
        self.send(&buf, flags)
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

    fn register_observer(
        &self,
        observer: Weak<dyn Observer<IoEvents>>,
        mask: IoEvents,
    ) -> Result<()> {
        let inner = self.state.write();
        match &*inner {
            State::Init(init) => init.register_observer(observer, mask),
            State::Listen(listen) => listen.register_observer(observer, mask),
            State::Connected(connected) => connected.register_observer(observer, mask),
        }
    }

    fn unregister_observer(
        &self,
        observer: &Weak<dyn Observer<IoEvents>>,
    ) -> Option<Weak<dyn Observer<IoEvents>>> {
        let inner = self.state.write();
        match &*inner {
            State::Init(init) => init.unregister_observer(observer),
            State::Listen(listen) => listen.unregister_observer(observer),
            State::Connected(connected) => connected.unregister_observer(observer),
        }
    }
}

impl Socket for UnixStreamSocket {
    fn bind(&self, socket_addr: SocketAddr) -> Result<()> {
        let addr = UnixSocketAddr::try_from(socket_addr)?;

        match &mut *self.state.write() {
            State::Init(init) => init.bind(addr),
            _ => return_errno_with_message!(
                Errno::EINVAL,
                "cannot bind a listening or connected socket"
            ),
            // FIXME: Maybe binding a connected socket should also be allowed?
        }
    }

    fn connect(&self, socket_addr: SocketAddr) -> Result<()> {
        let remote_addr = UnixSocketAddr::try_from(socket_addr)?.connect()?;

        // Note that the Linux kernel implementation locks the remote socket and checks to see if
        // it is listening first. This is different from our implementation, which locks the local
        // socket and checks the state of the local socket first.
        //
        // The difference may result in different error codes, but it's doubtful that this will
        // ever lead to real problems.
        //
        // See also <https://elixir.bootlin.com/linux/v6.10.4/source/net/unix/af_unix.c#L1527>.

        let client_addr = match &*self.state.read() {
            State::Init(init) => init.addr().cloned(),
            State::Listen(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is listening")
            }
            State::Connected(_) => {
                return_errno_with_message!(Errno::EISCONN, "the socket is connected")
            }
        };

        // We use the `push_incoming` directly to avoid holding the read lock of `self.state`
        // because it might call `Thread::yield_now` to wait for connection.
        loop {
            let res = push_incoming(&remote_addr, client_addr.clone());
            match res {
                Ok(connected) => {
                    *self.state.write() = State::Connected(connected);
                    return Ok(());
                }
                Err(err) if err.error() == Errno::EAGAIN => {
                    // FIXME: Calling `Thread::yield_now` can cause the thread to run when the backlog is full,
                    // which wastes a lot of CPU time. Using `WaitQueue` maybe a better solution.
                    Thread::yield_now()
                }
                Err(err) => return Err(err),
            }
        }
    }

    fn listen(&self, backlog: usize) -> Result<()> {
        let mut state = self.state.write();

        let addr = match &*state {
            State::Init(init) => init
                .addr()
                .ok_or(Error::with_message(
                    Errno::EINVAL,
                    "the socket is not bound",
                ))?
                .clone(),
            State::Listen(listen) => {
                return listen.listen(backlog);
            }
            State::Connected(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is connected")
            }
        };

        let listener = Listener::new(addr, backlog);
        *state = State::Listen(listener);

        Ok(())
    }

    fn accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        if self.is_nonblocking() {
            self.try_accept()
        } else {
            self.wait_events(IoEvents::IN, || self.try_accept())
        }
    }

    fn shutdown(&self, cmd: SockShutdownCmd) -> Result<()> {
        match &*self.state.read() {
            State::Connected(connected) => connected.shutdown(cmd),
            _ => return_errno_with_message!(Errno::ENOTCONN, "the socked is not connected"),
        }
    }

    fn addr(&self) -> Result<SocketAddr> {
        let addr = match &*self.state.read() {
            State::Init(init) => init.addr().cloned(),
            State::Listen(listen) => Some(listen.addr().clone()),
            State::Connected(connected) => connected.addr().cloned(),
        };

        Ok(addr.into())
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        let peer_addr = match &*self.state.read() {
            State::Connected(connected) => connected.peer_addr().cloned(),
            _ => return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected"),
        };

        Ok(peer_addr.into())
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
        let received_bytes = self.recv(&mut buf, flags)?;

        let copied_bytes = {
            let message = &buf[..received_bytes];
            copy_message_to_user(io_vecs, message)
        };

        // TODO: Receive control message

        let message_header = MessageHeader::new(None, None);

        Ok((copied_bytes, message_header))
    }
}
