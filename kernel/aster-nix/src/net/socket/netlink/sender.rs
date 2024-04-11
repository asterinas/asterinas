// SPDX-License-Identifier: MPL-2.0

use super::addr::NetlinkSocketAddr;
use crate::{fs::utils::Producer, prelude::*};

/// A netlink send endpoint.
#[derive(Clone)]
pub struct Sender {
    addr: Arc<NetlinkSocketAddr>,
    producer: Arc<Producer<NetlinkMessage>>,
}

impl Sender {
    pub(super) fn new(producer: Producer<NetlinkMessage>, addr: Arc<NetlinkSocketAddr>) -> Self {
        Self {
            addr,
            producer: Arc::new(producer),
        }
    }
}

#[derive(Debug)]
pub struct NetlinkMessage {
    src: NetlinkSocketAddr,
    msg: Box<[u8]>,
}
