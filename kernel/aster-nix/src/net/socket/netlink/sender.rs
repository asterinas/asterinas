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
    src: NetlinkSocketAddr,
    msg: Box<[u8]>,
}
