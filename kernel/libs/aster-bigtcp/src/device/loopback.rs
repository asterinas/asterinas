// SPDX-License-Identifier: MPL-2.0

use alloc::collections::VecDeque;
use core::fmt::{Debug, Formatter};

use super::{
    AnyNetworkDevice, ChecksumCapabilities, DeviceCapabilities, EthernetAddress, Medium, NetError,
};
use crate::packet::{AllocatedTxPacket, LinkLayer, RxPacket, TxPacket};

/// A loopback network device.
pub struct Loopback {
    queue: VecDeque<RxPacket<LinkLayer>>,
}

impl Loopback {
    /// Creates a loopback device.
    ///
    /// Every transmitted packet is made available for reception in FIFO order.
    #[expect(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
        }
    }
}

impl AnyNetworkDevice for Loopback {
    fn mac_addr(&self) -> EthernetAddress {
        EthernetAddress::default()
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut capabilities = DeviceCapabilities::default();
        capabilities.max_transmission_unit = 65535;
        capabilities.medium = Medium::Ip;
        capabilities.checksum = ChecksumCapabilities::ignored();
        capabilities
    }

    fn can_receive(&self) -> bool {
        !self.queue.is_empty()
    }

    fn can_send(&self) -> bool {
        true
    }

    fn receive(&mut self) -> Result<RxPacket<LinkLayer>, NetError> {
        self.queue.pop_front().ok_or(NetError::NotReady)
    }

    fn send(&mut self, packet: TxPacket<LinkLayer>) -> Result<(), NetError> {
        self.queue.push_back(packet.to_rx_packet());
        Ok(())
    }

    fn alloc_tx_buffer(payload_len: usize) -> Result<AllocatedTxPacket, ostd::Error> {
        AllocatedTxPacket::new(payload_len)
    }

    fn free_processed_tx_buffers(&mut self) {}

    fn notify_poll_end(&mut self) {}
}

impl Debug for Loopback {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Loopback")
            .field("queued_packets", &self.queue.len())
            .finish()
    }
}
