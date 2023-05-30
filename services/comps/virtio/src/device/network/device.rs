use core::hint::spin_loop;

use alloc::vec::Vec;
use jinux_frame::offset_of;
use jinux_pci::{capability::vendor::virtio::CapabilityVirtioData, util::BAR};
use jinux_util::{frame_ptr::InFramePtr, slot_vec::SlotVec};
use log::debug;
use pod::Pod;

use crate::{
    device::{network::config::NetworkFeatures, VirtioDeviceError},
    queue::{QueueError, VirtQueue},
    VirtioPciCommonCfg,
};

use super::{
    buffer::{RxBuffer, TxBuffer},
    config::VirtioNetConfig,
    header::VirtioNetHdr,
};

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct EthernetAddr(pub [u8; 6]);

#[derive(Debug, Clone, Copy)]
pub enum VirtioNetError {
    NotReady,
    WrongToken,
    Unknown,
}

pub struct NetworkDevice {
    config: VirtioNetConfig,
    mac_addr: EthernetAddr,
    send_queue: VirtQueue,
    recv_queue: VirtQueue,
    rx_buffers: SlotVec<RxBuffer>,
}

impl From<QueueError> for VirtioNetError {
    fn from(value: QueueError) -> Self {
        match value {
            QueueError::NotReady => VirtioNetError::NotReady,
            QueueError::WrongToken => VirtioNetError::WrongToken,
            _ => VirtioNetError::Unknown,
        }
    }
}

impl NetworkDevice {
    pub(crate) fn negotiate_features(device_features: u64) -> u64 {
        let device_features = NetworkFeatures::from_bits_truncate(device_features);
        let supported_features = NetworkFeatures::support_features();
        let network_features = device_features & supported_features;
        debug!("{:?}", network_features);
        network_features.bits()
    }

    pub fn new(
        cap: &CapabilityVirtioData,
        bars: [Option<BAR>; 6],
        common_cfg: &InFramePtr<VirtioPciCommonCfg>,
        notify_base_address: usize,
        notify_off_multiplier: u32,
        mut msix_vector_left: Vec<u16>,
    ) -> Result<Self, VirtioDeviceError> {
        let virtio_net_config = VirtioNetConfig::new(cap, bars);
        let features = {
            // select low
            common_cfg.write_at(
                offset_of!(VirtioPciCommonCfg, device_feature_select),
                0 as u32,
            );
            let device_feature_low =
                common_cfg.read_at(offset_of!(VirtioPciCommonCfg, device_feature)) as u64;
            // select high
            common_cfg.write_at(
                offset_of!(VirtioPciCommonCfg, device_feature_select),
                1 as u32,
            );
            let device_feature_high =
                common_cfg.read_at(offset_of!(VirtioPciCommonCfg, device_feature)) as u64;
            let device_feature = device_feature_high << 32 | device_feature_low;
            NetworkFeatures::from_bits_truncate(Self::negotiate_features(device_feature))
        };
        debug!("virtio_net_config = {:?}", virtio_net_config);
        debug!("features = {:?}", features);
        let mac_addr = virtio_net_config.read_at(offset_of!(VirtioNetConfig, mac));
        let status = virtio_net_config.read_at(offset_of!(VirtioNetConfig, status));
        debug!("mac addr = {:x?}, status = {:?}", mac_addr, status);
        let (recv_msix_vec, send_msix_vec) = {
            if msix_vector_left.len() >= 2 {
                let vector1 = msix_vector_left.pop().unwrap();
                let vector2 = msix_vector_left.pop().unwrap();
                (vector1, vector2)
            } else {
                let vector = msix_vector_left.pop().unwrap();
                (vector, vector)
            }
        };
        let mut recv_queue = VirtQueue::new(
            &common_cfg,
            QUEUE_RECV as usize,
            QUEUE_SIZE,
            notify_base_address,
            notify_off_multiplier,
            recv_msix_vec,
        )
        .expect("creating recv queue fails");
        let send_queue = VirtQueue::new(
            &common_cfg,
            QUEUE_SEND as usize,
            QUEUE_SIZE,
            notify_base_address,
            notify_off_multiplier,
            send_msix_vec,
        )
        .expect("create send queue fails");

        let mut rx_buffers = SlotVec::new();
        for i in 0..QUEUE_SIZE {
            let mut rx_buffer = RxBuffer::new(RX_BUFFER_LEN);
            let token = recv_queue.add(&[], &mut [rx_buffer.buf_mut()])?;
            assert_eq!(i, token);
            assert_eq!(rx_buffers.put(rx_buffer) as u16, i);
        }

        if recv_queue.should_notify() {
            debug!("notify receive queue");
            recv_queue.notify();
        }

        Ok(Self {
            config: virtio_net_config.read(),
            mac_addr,
            send_queue,
            recv_queue,
            rx_buffers,
        })
    }

    /// Add a rx buffer to recv queue
    fn add_rx_buffer(&mut self, mut rx_buffer: RxBuffer) -> Result<(), VirtioNetError> {
        let token = self.recv_queue.add(&[], &mut [rx_buffer.buf_mut()])?;
        assert!(self.rx_buffers.put_at(token as usize, rx_buffer).is_none());
        if self.recv_queue.should_notify() {
            self.recv_queue.notify();
        }
        Ok(())
    }

    // Acknowledge interrupt
    pub fn ack_interrupt(&self) -> bool {
        todo!()
    }

    /// The mac address
    pub fn mac_addr(&self) -> EthernetAddr {
        self.mac_addr
    }

    /// Send queue is ready
    pub fn can_send(&self) -> bool {
        self.send_queue.available_desc() >= 2
    }

    /// Receive queue is ready
    pub fn can_receive(&self) -> bool {
        self.recv_queue.can_pop()
    }

    /// Receive a packet from network. If packet is ready, returns a RxBuffer containing the packet.
    /// Otherwise, return NotReady error.
    pub fn receive(&mut self) -> Result<RxBuffer, VirtioNetError> {
        let (token, len) = self.recv_queue.pop_used()?;
        debug!("receive packet: token = {}, len = {}", token, len);
        let mut rx_buffer = self
            .rx_buffers
            .remove(token as usize)
            .ok_or(VirtioNetError::WrongToken)?;
        rx_buffer.set_packet_len(len as usize);
        // FIXME: Ideally, we can reuse the returned buffer without creating new buffer.
        // But this requires locking device to be compatible with smoltcp interface.
        let new_rx_buffer = RxBuffer::new(RX_BUFFER_LEN);
        self.add_rx_buffer(new_rx_buffer)?;
        Ok(rx_buffer)
    }

    /// Send a packet to network. Return until the request completes.
    pub fn send(&mut self, tx_buffer: TxBuffer) -> Result<(), VirtioNetError> {
        let header = VirtioNetHdr::default();
        let token = self
            .send_queue
            .add(&[header.as_bytes(), tx_buffer.buf()], &mut [])?;

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
            return Err(VirtioNetError::WrongToken);
        }
        debug!("send packet succeeds");
        Ok(())
    }
}

const QUEUE_RECV: u16 = 0;
const QUEUE_SEND: u16 = 1;

const QUEUE_SIZE: u16 = 64;
const RX_BUFFER_LEN: usize = 4096;
