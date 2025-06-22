// SPDX-License-Identifier: MPL-2.0

use core::marker::PhantomData;

use crate::{
    events::IoEvents,
    net::socket::{
        netlink::{
            common::bound::BoundNetlink,
            receiver::{MessageQueue, MessageReceiver},
            table::SupportedNetlinkProtocol,
            GroupIdSet, NetlinkSocketAddr,
        },
        util::datagram_common,
    },
    prelude::*,
    process::signal::Pollee,
};

pub(super) struct UnboundNetlink<P: SupportedNetlinkProtocol> {
    groups: GroupIdSet,
    phantom: PhantomData<BoundNetlink<P::Message>>,
}

impl<P: SupportedNetlinkProtocol> UnboundNetlink<P> {
    pub(super) const fn new() -> Self {
        Self {
            groups: GroupIdSet::new_empty(),
            phantom: PhantomData,
        }
    }

    pub(super) fn addr(&self) -> NetlinkSocketAddr {
        NetlinkSocketAddr::new(0, self.groups)
    }

    pub(super) fn add_groups(&mut self, groups: GroupIdSet) {
        self.groups.add_groups(groups);
    }

    pub(super) fn drop_groups(&mut self, groups: GroupIdSet) {
        self.groups.drop_groups(groups);
    }
}

impl<P: SupportedNetlinkProtocol> datagram_common::Unbound for UnboundNetlink<P> {
    type Endpoint = NetlinkSocketAddr;
    type BindOptions = ();

    type Bound = BoundNetlink<P::Message>;

    fn bind(
        &mut self,
        endpoint: &Self::Endpoint,
        pollee: &Pollee,
        _options: Self::BindOptions,
    ) -> Result<Self::Bound> {
        let message_queue = MessageQueue::<P::Message>::new();

        let bound_handle = {
            let endpoint = {
                let mut endpoint = *endpoint;
                endpoint.add_groups(self.groups);
                endpoint
            };
            let receiver = MessageReceiver::new(message_queue.clone(), pollee.clone());
            <P as SupportedNetlinkProtocol>::bind(&endpoint, receiver)?
        };

        Ok(BoundNetlink::new(bound_handle, message_queue))
    }

    fn bind_ephemeral(
        &mut self,
        _remote_endpoint: &Self::Endpoint,
        pollee: &Pollee,
    ) -> Result<Self::Bound> {
        let message_queue = MessageQueue::<P::Message>::new();

        let bound_handle = {
            let endpoint = {
                let mut endpoint = NetlinkSocketAddr::new_unspecified();
                endpoint.add_groups(self.groups);
                endpoint
            };
            let receiver = MessageReceiver::new(message_queue.clone(), pollee.clone());
            <P as SupportedNetlinkProtocol>::bind(&endpoint, receiver)?
        };

        Ok(BoundNetlink::new(bound_handle, message_queue))
    }

    fn check_io_events(&self) -> IoEvents {
        IoEvents::OUT
    }
}
