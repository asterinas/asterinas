// SPDX-License-Identifier: MPL-2.0

use super::addr::NetlinkSocketAddr;
use crate::{fs::utils::Producer, prelude::*, process::signal::CanPoll};

/// A netlink send endpoint.
#[derive(Clone)]
pub struct Sender {
    producer: Arc<Producer<NetlinkMessage>>,
}

impl Sender {
    pub(super) fn new(producer: Producer<NetlinkMessage>) -> Self {
        Self {
            producer: Arc::new(producer),
        }
    }

    pub fn send(&self, msg: NetlinkMessage) -> Result<()> {
        self.producer.push(msg).map_err(|(err, _)| err)
    }
}

impl CanPoll for Sender {
    fn poll_object(&self) -> &dyn CanPoll {
        self.producer.as_ref()
    }
}

#[derive(Debug, Clone)]
pub struct NetlinkMessage(Arc<MessageInner>);

#[derive(Debug)]
struct MessageInner {
    src_addr: NetlinkSocketAddr,
    msg: Box<[u8]>,
}

impl NetlinkMessage {
    pub fn new<T: Into<Box<[u8]>>>(src_addr: NetlinkSocketAddr, msg: T) -> Self {
        let inner = MessageInner {
            src_addr,
            msg: msg.into(),
        };
        Self(Arc::new(inner))
    }

    /// Returns the message source address
    pub fn src_addr(&self) -> NetlinkSocketAddr {
        self.0.src_addr
    }

    /// Copies the message to the `dst` buffer.
    /// Returns the number actually copied.
    ///
    /// If `dst.len()` is shorter that `self.len()`,
    /// the message will be truncated to fit the `dst` buffer,
    /// like the behavior of datagram socket.
    ///
    /// FIXME: further check the exact behavior of netlink,
    /// if `dst.len()` is shorter that `self.len()`.
    pub fn copy_to(&self, dst: &mut [u8]) -> usize {
        let copy_len = dst.len().min(self.len());
        dst[..copy_len].copy_from_slice(&self.0.msg[..copy_len]);
        copy_len
    }

    /// Returns the length of message
    pub fn len(&self) -> usize {
        self.0.msg.len()
    }
}
