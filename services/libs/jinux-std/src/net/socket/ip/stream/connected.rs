use core::sync::atomic::{AtomicBool, Ordering};

use crate::events::IoEvents;
use crate::net::iface::IpEndpoint;
use crate::process::signal::Poller;
use crate::{
    net::{
        iface::{AnyBoundSocket, RawTcpSocket},
        poll_ifaces,
        socket::util::{send_recv_flags::SendRecvFlags, shutdown_cmd::SockShutdownCmd},
    },
    prelude::*,
};

pub struct ConnectedStream {
    nonblocking: AtomicBool,
    bound_socket: Arc<AnyBoundSocket>,
    remote_endpoint: IpEndpoint,
}

impl ConnectedStream {
    pub fn new(
        is_nonblocking: bool,
        bound_socket: Arc<AnyBoundSocket>,
        remote_endpoint: IpEndpoint,
    ) -> Self {
        Self {
            nonblocking: AtomicBool::new(is_nonblocking),
            bound_socket,
            remote_endpoint,
        }
    }

    pub fn shutdown(&self, cmd: SockShutdownCmd) -> Result<()> {
        // TODO: deal with cmd
        self.bound_socket.raw_with(|socket: &mut RawTcpSocket| {
            socket.close();
        });
        poll_ifaces();
        Ok(())
    }

    pub fn recvfrom(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<(usize, IpEndpoint)> {
        debug_assert!(flags.is_all_supported());

        let poller = Poller::new();
        loop {
            let recv_len = self.try_recvfrom(buf, flags)?;
            if recv_len > 0 {
                let remote_endpoint = self.remote_endpoint()?;
                return Ok((recv_len, remote_endpoint));
            }
            let events = self.bound_socket.poll(IoEvents::IN, Some(&poller));
            if events.contains(IoEvents::HUP) || events.contains(IoEvents::ERR) {
                return_errno_with_message!(Errno::ENOTCONN, "recv packet fails");
            }
            if !events.contains(IoEvents::IN) {
                if self.is_nonblocking() {
                    return_errno_with_message!(Errno::EAGAIN, "try to recv again");
                }
                // FIXME: deal with receive timeout
                poller.wait()?;
            }
        }
    }

    fn try_recvfrom(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<usize> {
        poll_ifaces();
        let res = self.bound_socket.raw_with(|socket: &mut RawTcpSocket| {
            socket
                .recv_slice(buf)
                .map_err(|_| Error::with_message(Errno::ENOTCONN, "fail to recv packet"))
        });
        self.bound_socket.update_socket_state();
        res
    }

    pub fn sendto(&self, buf: &[u8], flags: SendRecvFlags) -> Result<usize> {
        debug_assert!(flags.is_all_supported());
        let mut sent_len = 0;
        let buf_len = buf.len();
        loop {
            let len = self
                .bound_socket
                .raw_with(|socket: &mut RawTcpSocket| socket.send_slice(&buf[sent_len..]))
                .map_err(|_| Error::with_message(Errno::ENOBUFS, "cannot send packet"))?;
            poll_ifaces();
            sent_len += len;
            if sent_len == buf_len {
                return Ok(sent_len);
            }
        }
    }

    pub fn local_endpoint(&self) -> Result<IpEndpoint> {
        self.bound_socket
            .local_endpoint()
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "does not has remote endpoint"))
    }

    pub fn remote_endpoint(&self) -> Result<IpEndpoint> {
        Ok(self.remote_endpoint)
    }

    pub fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.bound_socket.poll(mask, poller)
    }

    pub fn is_nonblocking(&self) -> bool {
        self.nonblocking.load(Ordering::Relaxed)
    }

    pub fn set_nonblocking(&self, nonblocking: bool) {
        self.nonblocking.store(nonblocking, Ordering::Relaxed);
    }
}
