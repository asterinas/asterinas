// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box, collections::linked_list::LinkedList, string::ToString, sync::Arc, vec::Vec,
};
use core::{fmt::Debug, mem::size_of};

use aster_bigtcp::device::{Checksum, DeviceCapabilities, Medium};
use aster_network::{
    AnyNetworkDevice, EthernetAddr, RxBuffer, TxBuffer, VirtioNetError, RX_BUFFER_POOL,
};
use aster_util::slot_vec::SlotVec;
use log::{debug, warn};
use ostd::{
    mm::DmaStream,
    sync::{LocalIrqDisabled, SpinLock},
    trap::TrapFrame,
};

use super::{config::VirtioNetConfig, header::VirtioNetHdr};
use crate::{
    device::{network::config::NetworkFeatures, VirtioDeviceError},
    queue::{QueueError, VirtQueue},
    transport::{ConfigManager, VirtioTransport},
};

pub struct NetworkDevice {
    config_manager: ConfigManager<VirtioNetConfig>,
    // For smoltcp use
    caps: DeviceCapabilities,
    mac_addr: EthernetAddr,
    send_queue: VirtQueue,
    recv_queue: VirtQueue,
    // Since the virtio net header remains consistent for each sending packet,
    // we store it to avoid recreating the header repeatedly.
    header: VirtioNetHdr,
    tx_buffers: Vec<Option<TxBuffer>>,
    rx_buffers: SlotVec<RxBuffer>,
    transport: Box<dyn VirtioTransport>,
}

impl NetworkDevice {
    pub(crate) fn negotiate_features(device_features: u64) -> u64 {
        let device_features = NetworkFeatures::from_bits_truncate(device_features);
        let supported_features = NetworkFeatures::support_features();
        let network_features = device_features & supported_features;

        if network_features != device_features {
            warn!(
                "Virtio net contains unsupported device features: {:?}",
                device_features.difference(supported_features)
            );
        }

        debug!("{:?}", network_features);
        network_features.bits()
    }

    pub fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let config_manager = VirtioNetConfig::new_manager(transport.as_ref());
        let config = config_manager.read_config();
        debug!("virtio_net_config = {:?}", config);
        let mac_addr = config.mac;
        let features = NetworkFeatures::from_bits_truncate(Self::negotiate_features(
            transport.read_device_features(),
        ));
        debug!("features = {:?}", features);

        let caps = init_caps(&features, &config);

        let mut send_queue = VirtQueue::new(QUEUE_SEND, QUEUE_SIZE, transport.as_mut())
            .expect("create send queue fails");
        send_queue.disable_callback();

        let mut recv_queue = VirtQueue::new(QUEUE_RECV, QUEUE_SIZE, transport.as_mut())
            .expect("creating recv queue fails");

        let tx_buffers = (0..QUEUE_SIZE).map(|_| None).collect();

        let mut rx_buffers = SlotVec::new();
        for i in 0..QUEUE_SIZE {
            let rx_pool = RX_BUFFER_POOL.get().unwrap();
            let rx_buffer = RxBuffer::new(size_of::<VirtioNetHdr>(), rx_pool);
            // FIEME: Replace rx_buffer with VM segment-based data structure to use dma mapping.
            let token = recv_queue.add_dma_buf(&[], &[&rx_buffer])?;
            assert_eq!(i, token);
            assert_eq!(rx_buffers.put(rx_buffer) as u16, i);
        }

        if recv_queue.should_notify() {
            debug!("notify receive queue");
            recv_queue.notify();
        }

        let mut device = Self {
            config_manager,
            caps,
            mac_addr,
            send_queue,
            recv_queue,
            header: VirtioNetHdr::default(),
            tx_buffers,
            rx_buffers,
            transport,
        };

        /// Interrupt handler if network device config space changes
        fn config_space_change(_: &TrapFrame) {
            debug!("network device config space change");
        }

        /// Interrupt handlers if network device receives/sends some packet
        fn handle_send_event(_: &TrapFrame) {
            aster_network::handle_send_irq(super::DEVICE_NAME);
        }
        fn handle_recv_event(_: &TrapFrame) {
            aster_network::handle_recv_irq(super::DEVICE_NAME);
        }

        device
            .transport
            .register_cfg_callback(Box::new(config_space_change))
            .unwrap();
        device
            .transport
            .register_queue_callback(QUEUE_SEND, Box::new(handle_send_event), true)
            .unwrap();
        device
            .transport
            .register_queue_callback(QUEUE_RECV, Box::new(handle_recv_event), true)
            .unwrap();

        device.transport.finish_init();

        aster_network::register_device(
            super::DEVICE_NAME.to_string(),
            Arc::new(SpinLock::new(device)),
        );
        Ok(())
    }

    /// Add a rx buffer to recv queue
    /// FIEME: Replace rx_buffer with VM segment-based data structure to use dma mapping.
    fn add_rx_buffer(&mut self, rx_buffer: RxBuffer) -> Result<(), VirtioNetError> {
        let token = self
            .recv_queue
            .add_dma_buf(&[], &[&rx_buffer])
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
        rx_buffer.set_packet_len(len as usize - size_of::<VirtioNetHdr>());
        // FIXME: Ideally, we can reuse the returned buffer without creating new buffer.
        // But this requires locking device to be compatible with smoltcp interface.
        let rx_pool = RX_BUFFER_POOL.get().unwrap();
        let new_rx_buffer = RxBuffer::new(size_of::<VirtioNetHdr>(), rx_pool);
        self.add_rx_buffer(new_rx_buffer)?;
        Ok(rx_buffer)
    }

    /// Send a packet to network. Return until the request completes.
    /// FIEME: Replace tx_buffer with VM segment-based data structure to use dma mapping.
    fn send(&mut self, packet: &[u8]) -> Result<(), VirtioNetError> {
        if !self.can_send() {
            return Err(VirtioNetError::Busy);
        }

        let tx_buffer = TxBuffer::new(&self.header, packet, &TX_BUFFER_POOL);

        let token = self
            .send_queue
            .add_dma_buf(&[&tx_buffer], &[])
            .map_err(queue_to_network_error)?;
        if self.send_queue.should_notify() {
            self.send_queue.notify();
        }

        debug!("send packet, token = {}, len = {}", token, packet.len());

        debug_assert!(self.tx_buffers[token as usize].is_none());
        self.tx_buffers[token as usize] = Some(tx_buffer);

        self.free_processed_tx_buffers();

        // If the send queue is not full, we can free the send buffers during the next sending process.
        // Therefore, there is no need to free the used buffers in the IRQ handlers.
        // This allows us to temporarily disable the send queue interrupt.
        // Conversely, if the send queue is full, the send queue interrupt should remain enabled
        // to free the send buffers as quickly as possible.
        if !self.can_send() {
            self.send_queue.enable_callback();
        } else {
            self.send_queue.disable_callback();
        }

        Ok(())
    }
}

fn queue_to_network_error(err: QueueError) -> VirtioNetError {
    match err {
        QueueError::NotReady => VirtioNetError::NotReady,
        QueueError::WrongToken => VirtioNetError::WrongToken,
        QueueError::BufferTooSmall => VirtioNetError::Busy,
        _ => VirtioNetError::Unknown,
    }
}

fn init_caps(features: &NetworkFeatures, config: &VirtioNetConfig) -> DeviceCapabilities {
    let mut caps = DeviceCapabilities::default();

    caps.max_burst_size = None;
    caps.medium = Medium::Ethernet;

    if features.contains(NetworkFeatures::VIRTIO_NET_F_MTU) {
        // If `VIRTIO_NET_F_MTU` is negotiated, the MTU is decided by the device.
        caps.max_transmission_unit = config.mtu as usize;
    } else {
        // We do not support these features,
        // so this asserts that they are _not_ negotiated.
        //
        // Without these features, the MTU is 1514 bytes per the virtio-net specification
        // (see "5.1.6.3 Setting Up Receive Buffers" and "5.1.6.2 Packet Transmission").
        assert!(
            !features.contains(NetworkFeatures::VIRTIO_NET_F_GUEST_TSO4)
                && !features.contains(NetworkFeatures::VIRTIO_NET_F_GUEST_TSO6)
                && !features.contains(NetworkFeatures::VIRTIO_NET_F_GUEST_UFO)
        );
        caps.max_transmission_unit = 1514;
    }

    // We do not support checksum offloading.
    // So the features must not be negotiated,
    // and we must deliver fully checksummed packets to the device
    // and validate all checksums for packets from the device.
    assert!(
        !features.contains(NetworkFeatures::VIRTIO_NET_F_CSUM)
            && !features.contains(NetworkFeatures::VIRTIO_NET_F_GUEST_CSUM)
    );
    caps.checksum.tcp = Checksum::Both;
    caps.checksum.udp = Checksum::Both;
    caps.checksum.ipv4 = Checksum::Both;
    caps.checksum.icmpv4 = Checksum::Both;

    caps
}

impl AnyNetworkDevice for NetworkDevice {
    fn mac_addr(&self) -> EthernetAddr {
        self.mac_addr
    }

    fn capabilities(&self) -> DeviceCapabilities {
        self.caps.clone()
    }

    fn can_receive(&self) -> bool {
        self.recv_queue.can_pop()
    }

    fn can_send(&self) -> bool {
        self.send_queue.available_desc() >= 1
    }

    fn receive(&mut self) -> Result<RxBuffer, VirtioNetError> {
        self.receive()
    }

    fn send(&mut self, packet: &[u8]) -> Result<(), VirtioNetError> {
        self.send(packet)
    }

    fn free_processed_tx_buffers(&mut self) {
        while let Ok((token, _)) = self.send_queue.pop_used() {
            self.tx_buffers[token as usize] = None;
        }
    }
}

impl Debug for NetworkDevice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NetworkDevice")
            .field("config", &self.config_manager.read_config())
            .field("mac_addr", &self.mac_addr)
            .field("send_queue", &self.send_queue)
            .field("recv_queue", &self.recv_queue)
            .field("transport", &self.transport)
            .finish()
    }
}

static TX_BUFFER_POOL: SpinLock<LinkedList<DmaStream>, LocalIrqDisabled> =
    SpinLock::new(LinkedList::new());

const QUEUE_RECV: u16 = 0;
const QUEUE_SEND: u16 = 1;

const QUEUE_SIZE: u16 = 64;
