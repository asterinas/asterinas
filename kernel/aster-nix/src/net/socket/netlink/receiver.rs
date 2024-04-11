// SPDX-License-Identifier: MPL-2.0

use super::{
    addr::NetlinkSocketAddr,
    sender::{NetlinkMessage, Sender},
};
use crate::{
    fs::utils::{Channel, Consumer, StatusFlags},
    prelude::*,
};

/// A netlink receive end.
pub struct Receiver {
    addr: Arc<NetlinkSocketAddr>,
    msgs: Consumer<NetlinkMessage>,
}

pub fn new_pair(is_nonblocking: bool, addr: NetlinkSocketAddr) -> Result<(Sender, Receiver)> {
    let flags = if is_nonblocking {
        StatusFlags::O_NONBLOCK
    } else {
        StatusFlags::empty()
    };

    let channel = Channel::with_capacity_and_flags(DEFAULT_CAPICITY, flags)?;

    let (producer, consumer) = channel.split();

    let addr = Arc::new(addr);
    let receiver = Receiver {
        msgs: consumer,
        addr: addr.clone(),
    };
    let sender = Sender::new(producer, addr);

    Ok((sender, receiver))
}

const DEFAULT_CAPICITY: usize = 16;
