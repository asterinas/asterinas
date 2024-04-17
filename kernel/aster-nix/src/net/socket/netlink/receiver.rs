// SPDX-License-Identifier: MPL-2.0

use super::sender::{NetlinkMessage, Sender};
use crate::{
    fs::utils::{Channel, Consumer, StatusFlags},
    prelude::*,
    process::signal::CanPoll,
};

/// A netlink receive end.
pub struct Receiver {
    msgs: Consumer<NetlinkMessage>,
}

impl Receiver {
    pub fn is_nonblocking(&self) -> bool {
        self.msgs.is_nonblocking()
    }

    pub fn set_nonblocking(&self) -> bool {
        let status_flags = self.msgs.status_flags() | StatusFlags::O_NONBLOCK;
        self.msgs.set_status_flags(status_flags);
    }

    pub fn receive(&self) {
        todo!()
    }
}

impl CanPoll for Receiver {
    fn poll_object(&self) -> &dyn CanPoll {
        &self.msgs
    }
}

pub fn new_pair(is_nonblocking: bool) -> Result<(Sender, Receiver)> {
    let flags = if is_nonblocking {
        StatusFlags::O_NONBLOCK
    } else {
        StatusFlags::empty()
    };

    let channel = Channel::with_capacity_and_flags(DEFAULT_CAPICITY, flags)?;

    let (producer, consumer) = channel.split();

    let receiver = Receiver { msgs: consumer };
    let sender = Sender::new(producer);

    Ok((sender, receiver))
}

const DEFAULT_CAPICITY: usize = 16;
