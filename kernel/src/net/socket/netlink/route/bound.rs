// SPDX-License-Identifier: MPL-2.0

use ostd::early_println;

use super::message::{
    AddrMessage, AnyResponseMessage, AttrOps, CAddrMessage, CMessageType, CNetlinkAttrHeader,
    CRtGenMessage, GetResponse, IfName, LinkAttrType, ReadAttrFromUser,
};
use crate::{
    events::IoEvents,
    net::socket::netlink::{
        message::CNetlinkMessageHeader,
        route::{
            kernel_socket::get_netlink_route_kernel,
            message::{AnyRequestMessage, CLinkMessage, GetRequest, LinkMessage, NlMsg},
        },
        table::BoundHandle,
        NetlinkSocketAddr,
    },
    prelude::*,
    util::{MultiRead, MultiWrite},
};

pub struct BoundNetlinkRoute {
    handle: BoundHandle,
    receive_queue: Mutex<VecDeque<Box<dyn AnyResponseMessage>>>,
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

    pub fn send(&self, reader: &mut dyn MultiRead) -> Result<usize> {
        let mut nlmsg = NlMsg::read_from_user(reader)?;

        early_println!("sent_size = {}", nlmsg.total_len());

        let local_port = self.addr().port();
        for segment in nlmsg.segments.iter_mut() {
            let header = segment.header_mut();
            if header.pid == 0 {
                header.pid = local_port;
            }
        }

        early_println!("nlmsg = {:?}", nlmsg);

        todo!()

        // let mut sent_size = 0;

        // let mut header = reader.read_val::<CNetlinkMessageHeader>()?;
        // if header.pid == 0 {
        //     let addr = self.addr();
        //     header.pid = addr.port();
        // }

        // sent_size += core::mem::size_of::<CNetlinkMessageHeader>();

        // let request = match CMessageType::try_from(header.type_)? {
        //     CMessageType::GETLINK => {
        //         let link_message = read_link_message_from_user(&header, reader)?;
        //         Box::new(GetRequest::new(header, link_message)) as Box<dyn AnyRequestMessage>
        //     }
        //     CMessageType::GETADDR => {
        //         let addr_message = read_addr_msg_from_user(&header, reader)?;
        //         Box::new(GetRequest::new(header, addr_message)) as Box<dyn AnyRequestMessage>
        //     }
        //     _ => todo!(),
        // };

        // early_println!("request  = {:?}", request);

        // get_netlink_route_kernel().request(request.as_ref(), |response| {
        //     self.receive_queue.lock().push_back(response);
        // })?;

        // Ok(sent_size)
    }

    pub fn try_receive(&self, writer: &mut dyn MultiWrite) -> Result<usize> {
        let mut receive_queue = self.receive_queue.lock();

        let Some(response) = receive_queue.pop_front() else {
            return_errno_with_message!(Errno::EAGAIN, "nothing to receive");
        };

        if let Some(get_response) = response.as_any().downcast_ref::<GetResponse>() {
            let received_len = get_response.write_to_user(writer)?;
            return Ok(received_len);
        }

        todo!()
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

// fn read_link_message_from_user(
//     header: &CNetlinkMessageHeader,
//     reader: &mut dyn MultiRead,
// ) -> Result<LinkMessage> {
//     let link_len = header.len as usize - core::mem::size_of_val(header);

//     // The actual message should be `CLinkMessage`,
//     // however, old Linux uses `CRtGenMessage` here.
//     // We should deal with both cases.
//     // Ref: https://elixir.bootlin.com/linux/v6.13/source/net/core/rtnetlink.c#L2393
//     let c_link_message = if link_len < core::mem::size_of::<CLinkMessage>() {
//         let legacy = reader.read_val::<CRtGenMessage>()?;
//         CLinkMessage::from(legacy)
//     } else {
//         reader.read_val::<CLinkMessage>()?
//     };

//     if c_link_message._pad != 0
//         || c_link_message.type_ != 0
//         || c_link_message.flags != 0
//         || c_link_message.change != 0
//     {
//         return_errno_with_message!(Errno::EINVAL, "invalid value for getlink")
//     }

//     let attrs = if link_len > size_of::<CLinkMessage>() {
//         let attr_len = link_len - size_of::<CLinkMessage>();
//         let attrs = read_link_attrs_from_user(attr_len, reader)?;
//         println!("attrs = {:?}", attrs);
//         attrs
//     } else {
//         Vec::new()
//     };

//     LinkMessage::try_from_c(c_link_message, attrs)
// }

// fn read_link_attrs_from_user(
//     mut attr_len: usize,
//     reader: &mut dyn MultiRead,
// ) -> Result<Vec<Box<dyn AttrOps>>> {
//     let mut res = Vec::new();

//     while attr_len > 0 {
//         let header = reader.read_val::<CNetlinkAttrHeader>()?;
//         match LinkAttrType::try_from(*header.type_())? {
//             LinkAttrType::IFNAME => {
//                 let attr = Box::new(IfName::read_from_user(reader, &header)?) as Box<dyn AttrOps>;
//                 attr_len -= attr.total_len_with_padding();
//                 res.push(attr);
//             }
//             _ => todo!("parse other link attr type"),
//         }
//     }

//     Ok(res)
// }

// fn read_addr_msg_from_user(
//     header: &CNetlinkMessageHeader,
//     reader: &mut dyn MultiRead,
// ) -> Result<AddrMessage> {
//     let addr_len = header.len as usize - size_of_val(header);

//     let c_addr_msg = if addr_len < core::mem::size_of::<CAddrMessage>() {
//         let legacy = reader.read_val::<CRtGenMessage>()?;
//         CAddrMessage::from(legacy)
//     } else {
//         reader.read_val::<CAddrMessage>()?
//     };

//     let attrs = if addr_len > size_of::<CAddrMessage>() {
//         todo!()
//     } else {
//         Vec::new()
//     };

//     AddrMessage::try_from_c(c_addr_msg, attrs)
// }
