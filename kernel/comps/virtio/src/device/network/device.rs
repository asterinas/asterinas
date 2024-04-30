// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, collections::VecDeque, string::ToString, sync::Arc, vec::Vec};
use core::{
    fmt::Debug,
    mem::size_of,
    sync::atomic::{AtomicBool, Ordering},
};

use aster_frame::{offset_of, sync::SpinLock, trap::TrapFrame, vm::VmReader};
use aster_network::{
    AnyNetworkDevice, EthernetAddr, NetDeviceIrqHandler, RxBuffer, TxBuffer, VirtioNetError,
};
use aster_util::{field_ptr, slot_vec::SlotVec};
use log::debug;
use smoltcp::phy::{DeviceCapabilities, Medium};

use super::{config::VirtioNetConfig, header::VirtioNetHdr};
use crate::{
    device::{network::config::NetworkFeatures, VirtioDeviceError},
    queue::{QueueError, VirtQueue},
    transport::VirtioTransport,
};

pub struct NetworkDevice {
    config: VirtioNetConfig,
    mac_addr: EthernetAddr,
    send_queue: VirtQueue,
    /// This flag is placed behind an Arc to facilitate sharing
    /// between the device and send IRQ handler.
    /// It is utilized to indicate if the send queue is full,
    /// which suggests that there might be someone waiting until the send queue becomes available.
    /// When this flag is set to true,
    /// the registered send IRQ callbacks will be executed.
    /// After receiving send IRQ , the flag will be reset to false.
    is_send_queue_full: Arc<AtomicBool>,
    recv_queue: VirtQueue,
    tx_buffers: Vec<Option<TxBuffer>>,
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

        let tx_buffers = (0..QUEUE_SIZE).map(|_| None).collect();

        let mut rx_buffers = SlotVec::with_capacity(QUEUE_SIZE as usize);
        for i in 0..QUEUE_SIZE {
            let rx_buffer = RxBuffer::new(size_of::<VirtioNetHdr>());
            let token = recv_queue.add_dma_buf(&[], &[&rx_buffer])?;
            assert_eq!(i, token);
            assert_eq!(rx_buffers.put(rx_buffer) as u16, i);
        }

        if recv_queue.should_notify() {
            debug!("notify receive queue");
            recv_queue.notify();
        }

        let is_send_queue_full = Arc::new(AtomicBool::new(false));
        let mut device = Self {
            config: virtio_net_config.read().unwrap(),
            mac_addr,
            send_queue,
            is_send_queue_full: is_send_queue_full.clone(),
            recv_queue,
            tx_buffers,
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
        fn handle_recv_event(_: &TrapFrame) {
            aster_network::handle_recv_irq(super::DEVICE_NAME);
        }

        // Handle irq if device sends some packet
        let handle_send_event = move |_: &TrapFrame| {
            if let Ok(true) = is_send_queue_full.compare_exchange(
                true,
                false,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                aster_network::handle_send_irq(super::DEVICE_NAME);
            }
        };

        device
            .transport
            .register_cfg_callback(Box::new(config_space_change))
            .unwrap();
        device
            .transport
            .register_queue_callback(QUEUE_RECV, Box::new(handle_recv_event), false)
            .unwrap();
        device
            .transport
            .register_queue_callback(QUEUE_SEND, Box::new(handle_send_event), false)
            .unwrap();

        aster_network::register_device(
            super::DEVICE_NAME.to_string(),
            Arc::new(SpinLock::new(device)),
        );
        Ok(())
    }

    /// Add a rx buffer to recv queue
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
        rx_buffer.set_packet_len(len as usize);
        // FIXME: Ideally, we can reuse the returned buffer without creating new buffer.
        // But this requires locking device to be compatible with smoltcp interface.
        let new_rx_buffer = RxBuffer::new(size_of::<VirtioNetHdr>());
        self.add_rx_buffer(new_rx_buffer)?;
        Ok(rx_buffer)
    }

    /// Send a packet to network.
    fn send(&mut self, packet: &mut VmReader) -> Result<(), VirtioNetError> {
        // Before calling this `send` function,
        // the user must check the device have at least one descriptor
        // by calling `can_send`.
        // Further, each packet should have smaller size (MTU=1536)
        // than the TX_BUFFER_SIZE(2048).
        // So the packet can always be sent successfully.
        debug_assert!(self.can_send());
        debug_assert!(packet.remain() <= MTU);

        let mut tx_buffers = Vec::with_capacity(1);

        while packet.has_remain() {
            if tx_buffers.is_empty() {
                for tx_buffer in self.free_processed_tx_buffers() {
                    tx_buffers.push(tx_buffer);
                }
            }

            let tx_buffer = if let Some(mut tx_buffer) = tx_buffers.pop() {
                tx_buffer.set_packet(packet);
                tx_buffer
            } else {
                let header = VirtioNetHdr::default();
                TxBuffer::new(&header, packet)
            };

            self.add_tx_buffer(tx_buffer)?;
        }

        Ok(())
    }

    fn add_tx_buffer(&mut self, tx_buffer: TxBuffer) -> Result<(), VirtioNetError> {
        let token = self
            .send_queue
            .add_dma_buf(&[&tx_buffer], &[])
            .map_err(queue_to_network_error)?;

        if self.send_queue.should_notify() {
            self.send_queue.notify();
        }

        let tx_buffer_slot = self
            .tx_buffers
            .get_mut(token as usize)
            .expect("invalid token");
        debug_assert!(tx_buffer_slot.is_none());
        *tx_buffer_slot = Some(tx_buffer);

        Ok(())
    }

    fn send_buffers(&mut self, mut tx_buffers: VecDeque<TxBuffer>) -> Result<(), VirtioNetError> {
        while let Some(tx_buffer) = tx_buffers.pop_front() {
            // Since these buffers are freed processed buffers,
            // they can always be added back to send queue.
            debug_assert!(self.can_send());
            self.add_tx_buffer(tx_buffer)?;
        }

        Ok(())
    }

    fn free_processed_tx_buffers(&mut self) -> Vec<TxBuffer> {
        let mut tx_buffers = Vec::new();
        while self.send_queue.can_pop() {
            let (token, _) = self.send_queue.pop_used().expect("fails to pop used token");
            debug_assert!(self.tx_buffers.get(token as usize).is_some());
            let mut tx_buffer = {
                let slot = self.tx_buffers.get_mut(token as usize).unwrap();
                slot.take().unwrap()
            };

            tx_buffer.clear_packet();
            debug_assert!(!tx_buffer.contains_packet());

            tx_buffers.push(tx_buffer);
        }
        tx_buffers
    }

    fn can_send(&self) -> bool {
        let res = self.tx_buffers.iter().any(|tx_buffer| tx_buffer.is_none());
        if !res {
            self.is_send_queue_full.store(true, Ordering::Relaxed);
        }
        res
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
        self.can_send()
    }

    fn receive(&mut self) -> Result<RxBuffer, VirtioNetError> {
        self.receive()
    }

    fn send(&mut self, packet: &mut VmReader) -> Result<(), VirtioNetError> {
        self.send(packet)
    }

    fn send_buffers(&mut self, buffers: VecDeque<TxBuffer>) -> Result<(), VirtioNetError> {
        self.send_buffers(buffers)
    }

    fn free_processed_tx_buffers(&mut self) -> Vec<TxBuffer> {
        self.free_processed_tx_buffers()
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
const MTU: usize = 1536;
