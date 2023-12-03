use alloc::{vec::Vec, boxed::Box, string::ToString, sync::Arc};
use jinux_frame::{offset_of, trap::TrapFrame, sync::SpinLock};
use jinux_util::{slot_vec::SlotVec, field_ptr};
use log::debug;
use pod::Pod;

use crate::{queue::{VirtQueue, QueueError}, device::{VirtioDeviceError, socket::{register_device, DEVICE_NAME}}, transport::{VirtioTransport}};

use super::{buffer::RxBuffer, config::{VirtioVsockConfig, VsockFeatures}, connect::{ConnectionInfo, VsockEvent}, header::{VirtioVsockHdr, VirtioVsockOp, VIRTIO_VSOCK_HDR_LEN}, error::SocketError, VsockDeviceIrqHandler};

const QUEUE_SIZE: u16 = 64;
const QUEUE_RECV: u16 = 0;
const QUEUE_SEND: u16 = 1;
const QUEUE_EVENT: u16 = 2;

/// The size in bytes of each buffer used in the RX virtqueue. This must be bigger than size_of::<VirtioVsockHdr>().
const RX_BUFFER_SIZE: usize = 512;

/// Low-level driver for a Virtio socket device.
pub struct SocketDevice {
    config: VirtioVsockConfig,
    guest_cid: u64,

    /// Virtqueue to receive packets.
    send_queue: VirtQueue,
    recv_queue: VirtQueue,
    event_queue: VirtQueue,

    rx_buffers: SlotVec<RxBuffer>,
    transport: Box<dyn VirtioTransport>,
    callbacks: Vec<Box<&'static VsockDeviceIrqHandler>>,
}

impl SocketDevice {
    pub fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let virtio_vsock_config = VirtioVsockConfig::new(transport.as_mut());
        debug!("virtio_vsock_config = {:?}", virtio_vsock_config);
        let guest_cid = 
            field_ptr!(&virtio_vsock_config, VirtioVsockConfig, guest_cid_low).read().unwrap() as u64
            | (field_ptr!(&virtio_vsock_config, VirtioVsockConfig, guest_cid_high).read().unwrap() as u64) << 32;

        let mut recv_queue = VirtQueue::new(QUEUE_RECV,QUEUE_SIZE,transport.as_mut())
            .expect("createing recv queue fails");
        let send_queue = VirtQueue::new(QUEUE_SEND,QUEUE_SIZE,transport.as_mut())
            .expect("creating send queue fails");
        let event_queue = VirtQueue::new(QUEUE_EVENT,QUEUE_SIZE,transport.as_mut())
            .expect("creating event queue fails");

        // Allocate and add buffers for the RX queue.
        let mut rx_buffers = SlotVec::new();
        for i in 0..QUEUE_SIZE {
            let mut rx_buffer = RxBuffer::new(RX_BUFFER_SIZE);
            let token = recv_queue.add(&[], &mut [rx_buffer.buf_mut()])?;
            assert_eq!(i, token);
            assert_eq!(rx_buffers.put(rx_buffer) as u16, i);
        }
        
        if recv_queue.should_notify() {
            debug!("notify receive queue");
            recv_queue.notify();
        }

        let mut device = Self{
            config: virtio_vsock_config.read().unwrap(),
            guest_cid,
            send_queue,
            recv_queue,
            event_queue,
            rx_buffers,
            transport,
            callbacks: Vec::new(),
        };

        // Interrupt handler if vsock device config space changes
        fn config_space_change(_: &TrapFrame){
            debug!("vsock device config space change");
        }

        // Interrupt handler if vsock device receives some packet.
        // TODO: This will be handled by vsock socket layer.
        fn handle_vsock_event(_: &TrapFrame){
            debug!("Packet received. This will be solved by socket layer");
        }

        device
            .transport
            .register_cfg_callback(Box::new(config_space_change))
            .unwrap();
        device 
            .transport
            .register_queue_callback(QUEUE_RECV, Box::new(handle_vsock_event), false)
            .unwrap();

        device.transport.finish_init();

        register_device(
            super::DEVICE_NAME.to_string(), 
            Arc::new(SpinLock::new(device)),
        );

        Ok(())
    }

    /// Returns the CID which has been assigned to this guest.
    pub fn guest_cid(&self) -> u64 {
        self.guest_cid
    }

    /// Sends a request to connect to the given destination.
    ///
    /// This returns as soon as the request is sent; you should wait until `poll` returns a
    /// [`VsockEventType::Connected`] event indicating that the peer has accepted the connection
    /// before sending data.
    pub fn connect(&mut self, connection_info: &ConnectionInfo) -> Result<(),SocketError> {
        let header = VirtioVsockHdr {
            op: VirtioVsockOp::Request as u16,
            ..connection_info.new_header(self.guest_cid)
        };
        // Sends a header only packet to the TX queue to connect the device to the listening socket
        // at the given destination.
        self.send_packet_to_tx_queue(&header, &[])
    }

    /// Accepts the given connection from a peer.
    pub fn accept(&mut self, connection_info: &ConnectionInfo) -> Result<(),SocketError> {
        let header = VirtioVsockHdr {
            op: VirtioVsockOp::Response as u16,
            ..connection_info.new_header(self.guest_cid)
        };
        self.send_packet_to_tx_queue(&header, &[])
    }

    /// Requests the peer to send us a credit update for the given connection.
    fn request_credit(&mut self, connection_info: &ConnectionInfo) -> Result<(),SocketError> {
        let header = VirtioVsockHdr {
            op: VirtioVsockOp::CreditRequest as u16,
            ..connection_info.new_header(self.guest_cid)
        };
        self.send_packet_to_tx_queue(&header, &[])
    }

    /// Tells the peer how much buffer space we have to receive data.
    pub fn credit_update(&mut self, connection_info: &ConnectionInfo) -> Result<(),SocketError> {
        let header = VirtioVsockHdr {
            op: VirtioVsockOp::CreditUpdate as u16,
            ..connection_info.new_header(self.guest_cid)
        };
        self.send_packet_to_tx_queue(&header, &[])
    }

    /// Requests to shut down the connection cleanly.
    ///
    /// This returns as soon as the request is sent; you should wait until `poll` returns a
    /// `VsockEventType::Disconnected` event if you want to know that the peer has acknowledged the
    /// shutdown.
    pub fn shutdown(&mut self, connection_info: &ConnectionInfo) -> Result<(),SocketError> {
        let header = VirtioVsockHdr {
            op: VirtioVsockOp::Shutdown as u16,
            ..connection_info.new_header(self.guest_cid)
        };
        self.send_packet_to_tx_queue(&header, &[])
    }

    /// Forcibly closes the connection without waiting for the peer.
    pub fn force_close(&mut self, connection_info: &ConnectionInfo) -> Result<(),SocketError> {
        let header = VirtioVsockHdr {
            op: VirtioVsockOp::Rst as u16,
            ..connection_info.new_header(self.guest_cid)
        };
        self.send_packet_to_tx_queue(&header, &[])
    }

    fn send_packet_to_tx_queue(&mut self, header: &VirtioVsockHdr, buffer: &[u8]) -> Result<(), SocketError> {
        let (_token, _len) = self.send_queue.add_notify_wait_pop(
            &[header.as_bytes(), buffer],
            &mut [],
        )?;
        // FORDEBUG
        // debug!("buffer in send_packet_to_tx_queue: {:?}",buffer);
        Ok(())
    }

    fn check_peer_buffer_is_sufficient(
        &mut self,
        connection_info: &mut ConnectionInfo,
        buffer_len: usize,
    ) -> Result<(), SocketError> {
        if connection_info.peer_free() as usize >= buffer_len {
            Ok(())
        } else {
            // Request an update of the cached peer credit, if we haven't already done so, and tell
            // the caller to try again later.
            if !connection_info.has_pending_credit_request {
                self.request_credit(connection_info)?;
                connection_info.has_pending_credit_request = true;
            }
            Err(SocketError::InsufficientBufferSpaceInPeer)
        }
    }

    /// Sends the buffer to the destination.
    pub fn send(&mut self, buffer: &[u8], connection_info: &mut ConnectionInfo) -> Result<(), SocketError> {
        self.check_peer_buffer_is_sufficient(connection_info, buffer.len())?;

        let len = buffer.len() as u32;
        let header = VirtioVsockHdr {
            op: VirtioVsockOp::Rw as u16,
            len: len,
            ..connection_info.new_header(self.guest_cid)
        };
        connection_info.tx_cnt += len;
        self.send_packet_to_tx_queue(&header, buffer)
    }

    /// Polls the RX virtqueue for the next event, and calls the given handler function to handle it.
    pub fn poll(&mut self, handler: impl FnOnce(VsockEvent, &[u8]) -> Result<Option<VsockEvent>,SocketError>
    ) -> Result<Option<VsockEvent>, SocketError> {
        // Return None if there is no pending packet.
        if !self.recv_queue.can_pop(){
            return Ok(None);
        }
        let (token, len) = self.recv_queue.pop_used()?;

        let mut buffer = self
            .rx_buffers
            .remove(token as usize)
            .ok_or(QueueError::WrongToken)?;

        let header = buffer.virtio_vsock_header();
        // The length written should be equal to len(header)+len(packet)
        assert_eq!(len, header.len() + VIRTIO_VSOCK_HDR_LEN as u32);

        buffer.set_packet_len(RX_BUFFER_SIZE);


        let head_result = read_header_and_body(&buffer.buf());

        let Ok((header,body)) = head_result else {
            let ret = match head_result {
                Err(e) => Err(e),
                _ => Ok(None) //FIXME: this clause is never reached.
            };
            self.add_rx_buffer(buffer, token)?;
            return ret;
        };

        debug!("Received packet {:?}. Op {:?}", header, header.op());
        debug!("body is {:?}",body);

        let result = VsockEvent::from_header(&header).and_then(|event| handler(event, body));

        // reuse the buffer and give it back to recv_queue.
        self.add_rx_buffer(buffer, token)?;

        result

    }

    /// Add a used rx buffer to recv queue,@index is only to check the correctness
    fn add_rx_buffer(&mut self, mut rx_buffer: RxBuffer, index: u16) -> Result<(), SocketError> {
        let token = self.recv_queue.add(&[], &mut [rx_buffer.buf_mut()])?;
        assert_eq!(index,token);
        assert!(self.rx_buffers.put_at(token as usize, rx_buffer).is_none());
        if self.recv_queue.should_notify() {
            self.recv_queue.notify();
        }
        Ok(())
    }

    /// Negotiate features for the device specified bits 0~23
    pub(crate) fn negotiate_features(features: u64) -> u64 {
        let device_features = VsockFeatures::from_bits_truncate(features);
        let supported_features = VsockFeatures::support_features();
        let vsock_features = device_features & supported_features;
        debug!("features negotiated: {:?}",vsock_features);
        vsock_features.bits()
    }

    
}

fn read_header_and_body(buffer: &[u8]) -> Result<(VirtioVsockHdr, &[u8]),SocketError> {
    // Shouldn't panic, because we know `RX_BUFFER_SIZE > size_of::<VirtioVsockHdr>()`.
    let header = VirtioVsockHdr::from_bytes(&buffer[..VIRTIO_VSOCK_HDR_LEN]);
    let body_length = header.len() as usize;

    // This could fail if the device returns an unreasonably long body length.
    let data_end = VIRTIO_VSOCK_HDR_LEN
        .checked_add(body_length)
        .ok_or(SocketError::InvalidNumber)?;
    // This could fail if the device returns a body length longer than the buffer we gave it.
    let data = buffer
        .get(VIRTIO_VSOCK_HDR_LEN..data_end)
        .ok_or(SocketError::BufferTooShort)?;
    Ok((header, data))
}