// SPDX-License-Identifier: MPL-2.0

use alloc::vec;
use smoltcp::{phy, time::Instant};

use crate::{
    buffer::{RxBuffer, TxBuffer},
    AnyNetworkDevice,
};

impl phy::Device for dyn AnyNetworkDevice {
    type RxToken<'a> = RxToken;
    type TxToken<'a> = TxToken<'a>;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        if self.can_receive() {
            let rx_buffer = self.receive().unwrap();
            Some((RxToken(rx_buffer), TxToken(self)))
        } else {
            None
        }
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        if self.can_send() {
            Some(TxToken(self))
        } else {
            None
        }
    }

    fn capabilities(&self) -> phy::DeviceCapabilities {
        self.capabilities()
    }
}
pub struct RxToken(RxBuffer);

impl phy::RxToken for RxToken {
    fn consume<R, F>(mut self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let packet_but = self.0.packet_mut();
        f(packet_but)
    }
}

pub struct TxToken<'a>(&'a mut dyn AnyNetworkDevice);

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
