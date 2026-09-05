// SPDX-License-Identifier: MPL-2.0

use core::fmt::Debug;

use super::{
    AnyNetworkDevice, ChecksumCapabilities, DeviceCapabilities, EthernetAddress, Medium, NetError,
};
use crate::packet::{FreshTxPacket, LinkLayer, RxPacket, TxPacket};

/// A loopback network device.
#[derive(Debug)]
pub struct Loopback {
    _private: (),
}

impl Loopback {
    /// Creates a loopback device.
    ///
    /// Every transmitted packet is made available for reception in FIFO order.
    #[expect(clippy::new_without_default)]
    pub fn new() -> Self {
        Self { _private: () }
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
        false
    }

    fn can_send(&self) -> bool {
        true
    }

    fn receive(&mut self) -> Result<RxPacket<LinkLayer>, NetError> {
        Err(NetError::NotReady)
    }

    fn send(&mut self, _packet: TxPacket<LinkLayer>) -> Result<(), NetError> {
        // We process packets that should loop back before reaching the loopback device. So this is
        // an unexpected packet and we will just discard it.
        Ok(())
    }

    fn alloc_tx_buffer(payload_len: usize) -> Result<FreshTxPacket, ostd::Error> {
        FreshTxPacket::alloc(payload_len)
    }

    fn free_processed_tx_buffers(&mut self) {}

    fn notify_poll_end(&mut self) {}
}
