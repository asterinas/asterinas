// SPDX-License-Identifier: MPL-2.0

use core::ops::Sub;

use crate::{
    events::IoEvents,
    net::socket::{
        netlink::{
            route::{kernel::get_netlink_route_kernel, message::NlMsg},
            table::BoundHandle,
            NetlinkSocketAddr,
        },
        SendRecvFlags,
    },
    prelude::*,
    util::{MultiRead, MultiWrite},
};

pub struct BoundNetlinkRoute {
    handle: BoundHandle,
    receive_queue: Mutex<VecDeque<NlMsg>>,
}

impl BoundNetlinkRoute {
    pub const fn new(handle: BoundHandle) -> Self {
        Self {
            handle,
            receive_queue: Mutex::new(VecDeque::new()),
        }
    }

    pub const fn addr(&self) -> NetlinkSocketAddr {
        self.handle.addr()
    }

    pub fn try_send(&self, reader: &mut dyn MultiRead, flags: SendRecvFlags) -> Result<usize> {
        // TODO: Deal with flags
        if !flags.is_all_supported() {
            warn!("unsupported flags: {:?}", flags);
        }

        let mut nlmsg = NlMsg::read_from(reader)?;

        let local_port = self.addr().port();
        for segment in nlmsg.segments_mut() {
            let header = segment.header_mut();
            if header.pid == 0 {
                header.pid = local_port;
            }
        }

        get_netlink_route_kernel().request(&nlmsg, |response| {
            self.receive_queue.lock().push_back(response);
        })?;

        Ok(nlmsg.total_len())
    }

    pub fn try_receive(&self, writer: &mut dyn MultiWrite, flags: SendRecvFlags) -> Result<usize> {
        if flags.sub(SendRecvFlags::MSG_PEEK) != SendRecvFlags::empty() {
            warn!("unsupported flags: {:?}", flags);
        }

        let mut receive_queue = self.receive_queue.lock();

        if !flags.contains(SendRecvFlags::MSG_PEEK) {
            // No MSG_PEEK
            let Some(response) = receive_queue.pop_front() else {
                return_errno_with_message!(Errno::EAGAIN, "nothing to receive");
            };

            response.write_to(writer)?;
            return Ok(response.total_len());
        }

        // MSG_PEEK
        let Some(response) = receive_queue.front() else {
            return_errno_with_message!(Errno::EAGAIN, "nothing to receive");
        };
        response.write_to(writer)?;
        Ok(response.total_len())
    }

    pub fn check_io_events(&self) -> IoEvents {
        let mut events = IoEvents::OUT;

        let receive_queue = self.receive_queue.lock();
        if !receive_queue.is_empty() {
            events |= IoEvents::IN;
        }

        events
    }
}
