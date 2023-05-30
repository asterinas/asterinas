use alloc::vec;
use smoltcp::phy::{self, Medium};

use crate::VirtioNet;
use jinux_virtio::device::network::{
    buffer::{RxBuffer, TxBuffer},
    device::NetworkDevice,
};

impl phy::Device for VirtioNet {
    type RxToken<'a> = RxToken;
    type TxToken<'a> = TxToken<'a>;

    fn receive(
        &mut self,
        _timestamp: smoltcp::time::Instant,
    ) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        if self.can_receive() {
            let device = self.device_mut();
            let rx_buffer = device.receive().unwrap();
            Some((RxToken(rx_buffer), TxToken(device)))
        } else {
            None
        }
    }

    fn transmit(&mut self, _timestamp: smoltcp::time::Instant) -> Option<Self::TxToken<'_>> {
        if self.can_send() {
            let device = self.device_mut();
            Some(TxToken(device))
        } else {
            None
        }
    }

    fn capabilities(&self) -> phy::DeviceCapabilities {
        let mut caps = phy::DeviceCapabilities::default();
        caps.max_transmission_unit = 1536;
        caps.max_burst_size = Some(1);
        caps.medium = Medium::Ethernet;
        caps
    }
}

pub struct RxToken(RxBuffer);

impl phy::RxToken for RxToken {
    fn consume<R, F>(mut self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let packet_but = self.0.packet_mut();
        let res = f(packet_but);
        res
    }
}

pub struct TxToken<'a>(&'a mut NetworkDevice);

impl<'a> phy::TxToken for TxToken<'a> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buffer = vec![0u8; len];
        let res = f(&mut buffer);
        let tx_buffer = TxBuffer::new(&buffer);
        self.0.send(tx_buffer).expect("Send packet failed");
        res
    }
}
