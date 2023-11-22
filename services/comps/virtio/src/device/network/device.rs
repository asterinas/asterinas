use core::{fmt::Debug, hint::spin_loop, mem::size_of};

use alloc::{boxed::Box, string::ToString, sync::Arc, vec::Vec};
use aster_frame::{offset_of, sync::SpinLock, trap::TrapFrame};
use aster_network::{
    buffer::{RxBuffer, TxBuffer},
    AnyNetworkDevice, EthernetAddr, NetDeviceIrqHandler, VirtioNetError,
};
use aster_util::{field_ptr, slot_vec::SlotVec};
use log::debug;
use pod::Pod;
use smoltcp::phy::{DeviceCapabilities, Medium};

use crate::{
    device::{network::config::NetworkFeatures, VirtioDeviceError},
    queue::{QueueError, VirtQueue},
    transport::VirtioTransport,
};

use super::{config::VirtioNetConfig, header::VirtioNetHdr};

pub struct NetworkDevice {
    config: VirtioNetConfig,
    mac_addr: EthernetAddr,
    send_queue: VirtQueue,
    recv_queue: VirtQueue,
    rx_buffers: SlotVec<RxBuffer>,
    callbacks: Vec<Box<dyn NetDeviceIrqHandler>>,
    transport: Box<dyn VirtioTransport>,
}

impl NetworkDevice {
    pub(crate) fn negotiate_features(device_features: u64) -> u64 {
        let device_features = NetworkFeatures::from_bits_truncate(device_features);
        let supported_features = NetworkFeatures::support_features();
        let network_features = device_features & supported_features;
        debug!("{:?}", network_features);
        network_features.bits()
    }

    pub fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let virtio_net_config = VirtioNetConfig::new(transport.as_mut());
        let features = NetworkFeatures::from_bits_truncate(Self::negotiate_features(
            transport.device_features(),
        ));
        debug!("virtio_net_config = {:?}", virtio_net_config);
        debug!("features = {:?}", features);
        let mac_addr = field_ptr!(&virtio_net_config, VirtioNetConfig, mac)
            .read()
            .unwrap();
        let status = field_ptr!(&virtio_net_config, VirtioNetConfig, status)
            .read()
            .unwrap();
        debug!("mac addr = {:x?}, status = {:?}", mac_addr, status);
        let mut recv_queue = VirtQueue::new(QUEUE_RECV, QUEUE_SIZE, transport.as_mut())
            .expect("creating recv queue fails");
        let send_queue = VirtQueue::new(QUEUE_SEND, QUEUE_SIZE, transport.as_mut())
            .expect("create send queue fails");

        let mut rx_buffers = SlotVec::new();
        for i in 0..QUEUE_SIZE {
            let mut rx_buffer = RxBuffer::new(RX_BUFFER_LEN, size_of::<VirtioNetHdr>());
            // FIEME: Replace rx_buffer with VM segment-based data structure to use dma mapping.
            let token = recv_queue.add_buf(&[], &[rx_buffer.buf_mut()])?;
            assert_eq!(i, token);
            assert_eq!(rx_buffers.put(rx_buffer) as u16, i);
        }

        if recv_queue.should_notify() {
            debug!("notify receive queue");
            recv_queue.notify();
        }
        let mut device = Self {
            config: virtio_net_config.read().unwrap(),
            mac_addr,
            send_queue,
            recv_queue,
            rx_buffers,
            transport,
            callbacks: Vec::new(),
        };
        device.transport.finish_init();
        /// Interrupt handler if network device config space changes
        fn config_space_change(_: &TrapFrame) {
            debug!("network device config space change");
        }

        /// Interrupt handler if network device receives some packet
        fn handle_network_event(_: &TrapFrame) {
            aster_network::handle_recv_irq(super::DEVICE_NAME);
        }

        device
            .transport
            .register_cfg_callback(Box::new(config_space_change))
            .unwrap();
        device
            .transport
            .register_queue_callback(QUEUE_RECV, Box::new(handle_network_event), false)
            .unwrap();

        aster_network::register_device(
            super::DEVICE_NAME.to_string(),
            Arc::new(SpinLock::new(Box::new(device))),
        );
        Ok(())
    }

    /// Add a rx buffer to recv queue
    /// FIEME: Replace rx_buffer with VM segment-based data structure to use dma mapping.
    fn add_rx_buffer(&mut self, mut rx_buffer: RxBuffer) -> Result<(), VirtioNetError> {
        let token = self
            .recv_queue
            .add_buf(&[], &[rx_buffer.buf_mut()])
            .map_err(queue_to_network_error)?;
        assert!(self.rx_buffers.put_at(token as usize, rx_buffer).is_none());
        if self.recv_queue.should_notify() {
            self.recv_queue.notify();
        }
        Ok(())
    }

    /// Receive a packet from network. If packet is ready, returns a RxBuffer containing the packet.
    /// Otherwise, return NotReady error.
    fn receive(&mut self) -> Result<RxBuffer, VirtioNetError> {
        let (token, len) = self.recv_queue.pop_used().map_err(queue_to_network_error)?;
        debug!("receive packet: token = {}, len = {}", token, len);
        let mut rx_buffer = self
            .rx_buffers
            .remove(token as usize)
            .ok_or(VirtioNetError::WrongToken)?;
        rx_buffer.set_packet_len(len as usize);
        // FIXME: Ideally, we can reuse the returned buffer without creating new buffer.
        // But this requires locking device to be compatible with smoltcp interface.
        let new_rx_buffer = RxBuffer::new(RX_BUFFER_LEN, size_of::<VirtioNetHdr>());
        self.add_rx_buffer(new_rx_buffer)?;
        Ok(rx_buffer)
    }

    /// Send a packet to network. Return until the request completes.
    /// FIEME: Replace tx_buffer with VM segment-based data structure to use dma mapping.
    fn send(&mut self, tx_buffer: TxBuffer) -> Result<(), VirtioNetError> {
        let header = VirtioNetHdr::default();
        let token = self
            .send_queue
            .add_buf(&[header.as_bytes(), tx_buffer.buf()], &[])
            .map_err(queue_to_network_error)?;

        if self.send_queue.should_notify() {
            self.send_queue.notify();
        }
        // Wait until the buffer is used
        while !self.send_queue.can_pop() {
            spin_loop();
        }
        // Pop out the buffer, so we can reuse the send queue further
        let (pop_token, _) = self.send_queue.pop_used().map_err(queue_to_network_error)?;
        debug_assert!(pop_token == token);
        if pop_token != token {
            return Err(VirtioNetError::WrongToken);
        }
        debug!("send packet succeeds");
        Ok(())
    }
}

fn queue_to_network_error(err: QueueError) -> VirtioNetError {
    match err {
        QueueError::NotReady => VirtioNetError::NotReady,
        QueueError::WrongToken => VirtioNetError::WrongToken,
        _ => VirtioNetError::Unknown,
    }
}

impl AnyNetworkDevice for NetworkDevice {
    fn mac_addr(&self) -> EthernetAddr {
        self.mac_addr
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.max_transmission_unit = 1536;
        caps.max_burst_size = Some(1);
        caps.medium = Medium::Ethernet;
        caps
    }

    fn can_receive(&self) -> bool {
        self.recv_queue.can_pop()
    }

    fn can_send(&self) -> bool {
        self.send_queue.available_desc() >= 2
    }

    fn receive(&mut self) -> Result<RxBuffer, VirtioNetError> {
        self.receive()
    }

    fn send(&mut self, tx_buffer: TxBuffer) -> Result<(), VirtioNetError> {
        self.send(tx_buffer)
    }
}

impl Debug for NetworkDevice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NetworkDevice")
            .field("config", &self.config)
            .field("mac_addr", &self.mac_addr)
            .field("send_queue", &self.send_queue)
            .field("recv_queue", &self.recv_queue)
            .field("transport", &self.transport)
            .finish()
    }
}

const QUEUE_RECV: u16 = 0;
const QUEUE_SEND: u16 = 1;

const QUEUE_SIZE: u16 = 64;
const RX_BUFFER_LEN: usize = 4096;
