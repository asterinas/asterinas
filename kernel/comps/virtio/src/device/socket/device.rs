// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, string::ToString, sync::Arc, vec, vec::Vec};
use core::{fmt::Debug, hint::spin_loop, mem::size_of};

use aster_network::{RxBuffer, TxBuffer};
use aster_util::{field_ptr, slot_vec::SlotVec};
use log::debug;
use ostd::{mm::VmWriter, offset_of, sync::SpinLock, trap::TrapFrame, Pod};

use super::{
    config::{VirtioVsockConfig, VsockFeatures},
    connect::{ConnectionInfo, VsockEvent},
    error::SocketError,
    header::{VirtioVsockHdr, VirtioVsockOp, VIRTIO_VSOCK_HDR_LEN},
    VsockDeviceIrqHandler,
};
use crate::{
    device::{
        socket::{
            buffer::{RX_BUFFER_POOL, TX_BUFFER_POOL},
            handle_recv_irq, register_device,
        },
        VirtioDeviceError,
    },
    queue::{QueueError, VirtQueue},
    transport::VirtioTransport,
};

const QUEUE_SIZE: u16 = 64;
const QUEUE_RECV: u16 = 0;
const QUEUE_SEND: u16 = 1;
const QUEUE_EVENT: u16 = 2;

/// Vsock device driver
pub struct SocketDevice {
    config: VirtioVsockConfig,
    guest_cid: u64,

    /// Virtqueue to receive packets.
    send_queue: VirtQueue,
    recv_queue: VirtQueue,
    event_queue: VirtQueue,

    rx_buffers: SlotVec<RxBuffer>,
    transport: Box<dyn VirtioTransport>,
    callbacks: Vec<Box<dyn VsockDeviceIrqHandler>>,
}

impl SocketDevice {
    /// Create a new vsock device
    pub fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let virtio_vsock_config = VirtioVsockConfig::new(transport.as_mut());
        debug!("virtio_vsock_config = {:?}", virtio_vsock_config);
        let guest_cid = field_ptr!(&virtio_vsock_config, VirtioVsockConfig, guest_cid_low)
            .read_once()
            .unwrap() as u64
            | (field_ptr!(&virtio_vsock_config, VirtioVsockConfig, guest_cid_high)
                .read_once()
                .unwrap() as u64)
                << 32;

        let mut recv_queue = VirtQueue::new(QUEUE_RECV, QUEUE_SIZE, transport.as_mut())
            .expect("creating recv queue fails");
        let send_queue = VirtQueue::new(QUEUE_SEND, QUEUE_SIZE, transport.as_mut())
            .expect("creating send queue fails");
        let event_queue = VirtQueue::new(QUEUE_EVENT, QUEUE_SIZE, transport.as_mut())
            .expect("creating event queue fails");

        // Allocate and add buffers for the RX queue.
        let mut rx_buffers = SlotVec::new();
        for i in 0..QUEUE_SIZE {
            let rx_pool = RX_BUFFER_POOL.get().unwrap();
            let rx_buffer = RxBuffer::new(size_of::<VirtioVsockHdr>(), rx_pool);
            let token = recv_queue.add_dma_buf(&[], &[&rx_buffer])?;
            assert_eq!(i, token);
            assert_eq!(rx_buffers.put(rx_buffer) as u16, i);
        }

        if recv_queue.should_notify() {
            debug!("notify receive queue");
            recv_queue.notify();
        }

        let mut device = Self {
            config: virtio_vsock_config.read_once().unwrap(),
            guest_cid,
            send_queue,
            recv_queue,
            event_queue,
            rx_buffers,
            transport,
            callbacks: Vec::new(),
        };

        // Interrupt handler if vsock device config space changes
        fn config_space_change(_: &TrapFrame) {
            debug!("vsock device config space change");
        }

        // Interrupt handler if vsock device receives some packet.
        fn handle_vsock_event(_: &TrapFrame) {
            handle_recv_irq(super::DEVICE_NAME);
        }
        // FIXME: handle event virtqueue notification in live migration

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

    /// Return the CID which has been assigned to this guest.
    pub fn guest_cid(&self) -> u64 {
        self.guest_cid
    }

    /// Send a connection request
    pub fn request(&mut self, connection_info: &ConnectionInfo) -> Result<(), SocketError> {
        let header = VirtioVsockHdr {
            op: VirtioVsockOp::Request as u16,
            ..connection_info.new_header(self.guest_cid)
        };

        self.send_packet_to_tx_queue(&header, &[])
    }

    /// Send a response to peer, if peer start a sending request
    pub fn response(&mut self, connection_info: &ConnectionInfo) -> Result<(), SocketError> {
        let header = VirtioVsockHdr {
            op: VirtioVsockOp::Response as u16,
            ..connection_info.new_header(self.guest_cid)
        };
        self.send_packet_to_tx_queue(&header, &[])
    }

    /// Send a shutdown request
    pub fn shutdown(&mut self, connection_info: &ConnectionInfo) -> Result<(), SocketError> {
        let header = VirtioVsockHdr {
            op: VirtioVsockOp::Shutdown as u16,
            ..connection_info.new_header(self.guest_cid)
        };
        self.send_packet_to_tx_queue(&header, &[])
    }

    /// Send a reset request to peer
    pub fn reset(&mut self, connection_info: &ConnectionInfo) -> Result<(), SocketError> {
        let header = VirtioVsockHdr {
            op: VirtioVsockOp::Rst as u16,
            ..connection_info.new_header(self.guest_cid)
        };
        self.send_packet_to_tx_queue(&header, &[])
    }

    /// Request the peer to send the credit info to us
    pub fn credit_request(&mut self, connection_info: &ConnectionInfo) -> Result<(), SocketError> {
        let header = VirtioVsockHdr {
            op: VirtioVsockOp::CreditRequest as u16,
            ..connection_info.new_header(self.guest_cid)
        };
        self.send_packet_to_tx_queue(&header, &[])
    }

    /// Tell the peer our credit info
    pub fn credit_update(&mut self, connection_info: &ConnectionInfo) -> Result<(), SocketError> {
        let header = VirtioVsockHdr {
            op: VirtioVsockOp::CreditUpdate as u16,
            ..connection_info.new_header(self.guest_cid)
        };
        self.send_packet_to_tx_queue(&header, &[])
    }

    fn send_packet_to_tx_queue(
        &mut self,
        header: &VirtioVsockHdr,
        buffer: &[u8],
    ) -> Result<(), SocketError> {
        debug!("Sent packet {:?}. Op {:?}", header, header.op());
        debug!("buffer in send_packet_to_tx_queue: {:?}", buffer);
        let tx_buffer = {
            let pool = TX_BUFFER_POOL.get().unwrap();
            TxBuffer::new(header, buffer, pool)
        };

        let token = self.send_queue.add_dma_buf(&[&tx_buffer], &[])?;

        if self.send_queue.should_notify() {
            self.send_queue.notify();
        }

        // Wait until the buffer is used
        while !self.send_queue.can_pop() {
            spin_loop();
        }

        // Pop out the buffer, so we can reuse the send queue further
        let (pop_token, _) = self.send_queue.pop_used()?;
        debug_assert!(pop_token == token);
        if pop_token != token {
            return Err(SocketError::QueueError(QueueError::WrongToken));
        }
        debug!("send packet succeeds");
        Ok(())
    }

    fn check_peer_buffer_is_sufficient(
        &mut self,
        connection_info: &mut ConnectionInfo,
        buffer_len: usize,
    ) -> Result<(), SocketError> {
        debug!("connection info {:?}", connection_info);
        debug!(
            "peer free from peer: {:?}, buffer len : {:?}",
            connection_info.peer_free(),
            buffer_len
        );
        if connection_info.peer_free() as usize >= buffer_len {
            Ok(())
        } else {
            // Request an update of the cached peer credit, if we haven't already done so, and tell
            // the caller to try again later.
            if !connection_info.has_pending_credit_request {
                self.credit_request(connection_info)?;
                connection_info.has_pending_credit_request = true;
                //TODO check if the update needed
            }
            Err(SocketError::InsufficientBufferSpaceInPeer)
        }
    }

    /// Sends the buffer to the destination.
    pub fn send(
        &mut self,
        buffer: &[u8],
        connection_info: &mut ConnectionInfo,
    ) -> Result<(), SocketError> {
        self.check_peer_buffer_is_sufficient(connection_info, buffer.len())?;

        let len = buffer.len() as u32;
        let header = VirtioVsockHdr {
            op: VirtioVsockOp::Rw as u16,
            len,
            ..connection_info.new_header(self.guest_cid)
        };
        connection_info.tx_cnt += len;
        self.send_packet_to_tx_queue(&header, buffer)
    }

    /// Receive bytes from peer, returns the header
    pub fn receive(
        &mut self,
        // connection_info: &mut ConnectionInfo,
    ) -> Result<RxBuffer, SocketError> {
        let (token, len) = self.recv_queue.pop_used()?;
        debug!(
            "receive packet in rx_queue: token = {}, len = {}",
            token, len
        );
        let mut rx_buffer = self
            .rx_buffers
            .remove(token as usize)
            .ok_or(QueueError::WrongToken)?;
        rx_buffer.set_packet_len(len as usize);

        let rx_pool = RX_BUFFER_POOL.get().unwrap();
        let new_rx_buffer = RxBuffer::new(size_of::<VirtioVsockHdr>(), rx_pool);
        self.add_rx_buffer(new_rx_buffer, token)?;

        Ok(rx_buffer)
    }

    /// Polls the RX virtqueue for the next event, and calls the given handler function to handle it.
    pub fn poll(
        &mut self,
        handler: impl FnOnce(VsockEvent, &[u8]) -> Result<Option<VsockEvent>, SocketError>,
    ) -> Result<Option<VsockEvent>, SocketError> {
        // Return None if there is no pending packet.
        if !self.recv_queue.can_pop() {
            return Ok(None);
        }
        let rx_buffer = self.receive()?;

        let mut buf_reader = rx_buffer.buf();
        let mut temp_buffer = vec![0u8; buf_reader.remain()];
        buf_reader.read(&mut VmWriter::from(&mut temp_buffer as &mut [u8]));

        let (header, payload) = read_header_and_body(&temp_buffer)?;
        // The length written should be equal to len(header)+len(packet)
        debug!("Received packet {:?}. Op {:?}", header, header.op());
        debug!("body is {:?}", payload);
        VsockEvent::from_header(&header).and_then(|event| handler(event, payload))
    }

    /// Add a used rx buffer to recv queue,@index is only to check the correctness
    fn add_rx_buffer(&mut self, rx_buffer: RxBuffer, index: u16) -> Result<(), SocketError> {
        let token = self.recv_queue.add_dma_buf(&[], &[&rx_buffer])?;
        assert_eq!(index, token);
        assert!(self.rx_buffers.put_at(token as usize, rx_buffer).is_none());
        if self.recv_queue.should_notify() {
            self.recv_queue.notify();
        }
        Ok(())
    }

    /// Negotiate features for the device specified bits 0~23
    pub(crate) fn negotiate_features(features: u64) -> u64 {
        let device_features = VsockFeatures::from_bits_truncate(features);
        let supported_features = VsockFeatures::supported_features();
        let vsock_features = device_features & supported_features;
        debug!("features negotiated: {:?}", vsock_features);
        vsock_features.bits()
    }
}

impl Debug for SocketDevice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SocketDevice")
            .field("config", &self.config)
            .field("guest_cid", &self.guest_cid)
            .field("send_queue", &self.send_queue)
            .field("recv_queue", &self.recv_queue)
            .field("event_queue", &self.event_queue)
            .field("transport", &self.transport)
            .finish()
    }
}

fn read_header_and_body(buffer: &[u8]) -> Result<(VirtioVsockHdr, &[u8]), SocketError> {
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
