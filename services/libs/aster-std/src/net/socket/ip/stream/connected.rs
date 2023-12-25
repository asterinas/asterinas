use core::sync::atomic::{AtomicBool, Ordering};

use crate::events::{IoEvents, Observer};
use crate::net::iface::IpEndpoint;
use crate::process::signal::{Pollee, Poller};
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
    pollee: Pollee,
}

impl ConnectedStream {
    pub fn new(
        is_nonblocking: bool,
        bound_socket: Arc<AnyBoundSocket>,
        remote_endpoint: IpEndpoint,
        pollee: Pollee,
    ) -> Arc<Self> {
        let connected = Arc::new(Self {
            nonblocking: AtomicBool::new(is_nonblocking),
            bound_socket,
            remote_endpoint,
            pollee,
        });
        connected
            .bound_socket
            .set_observer(Arc::downgrade(&connected) as _);
        connected
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
            let events = self.poll(IoEvents::IN, Some(&poller));
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
        self.update_io_events();
        res
    }

    pub fn sendto(&self, buf: &[u8], flags: SendRecvFlags) -> Result<usize> {
        debug_assert!(flags.is_all_supported());

        let poller = Poller::new();
        loop {
            let sent_len = self.try_sendto(buf, flags)?;
            if sent_len > 0 {
                return Ok(sent_len);
            }
            let events = self.poll(IoEvents::OUT, Some(&poller));
            if events.contains(IoEvents::HUP) || events.contains(IoEvents::ERR) {
                return_errno_with_message!(Errno::ENOBUFS, "fail to send packets");
            }
            if !events.contains(IoEvents::OUT) {
                if self.is_nonblocking() {
                    return_errno_with_message!(Errno::EAGAIN, "try to send again");
                }
                // FIXME: deal with send timeout
                poller.wait()?;
            }
        }
    }

    fn try_sendto(&self, buf: &[u8], flags: SendRecvFlags) -> Result<usize> {
        let res = self
            .bound_socket
            .raw_with(|socket: &mut RawTcpSocket| socket.send_slice(buf))
            .map_err(|_| Error::with_message(Errno::ENOBUFS, "cannot send packet"));
        match res {
            // We have to explicitly invoke `update_io_events` when the send buffer becomes
            // full. Note that smoltcp does not think it is an interface event, so calling
            // `poll_ifaces` alone is not enough.
            Ok(0) => self.update_io_events(),
            Ok(_) => poll_ifaces(),
            _ => (),
        };
        res
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
        self.pollee.poll(mask, poller)
    }

    fn update_io_events(&self) {
        self.bound_socket.raw_with(|socket: &mut RawTcpSocket| {
            let pollee = &self.pollee;

            if socket.can_recv() {
                pollee.add_events(IoEvents::IN);
            } else {
                pollee.del_events(IoEvents::IN);
            }

            if socket.can_send() {
                pollee.add_events(IoEvents::OUT);
            } else {
                pollee.del_events(IoEvents::OUT);
            }
        });
    }

    pub fn is_nonblocking(&self) -> bool {
        self.nonblocking.load(Ordering::Relaxed)
    }

    pub fn set_nonblocking(&self, nonblocking: bool) {
        self.nonblocking.store(nonblocking, Ordering::Relaxed);
    }
}

impl Observer<()> for ConnectedStream {
    fn on_events(&self, _: &()) {
        self.update_io_events();
    }
}
