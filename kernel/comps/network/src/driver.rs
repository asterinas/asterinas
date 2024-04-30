// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::VecDeque, vec, vec::Vec};

use aster_frame::vm::{VmReader, VmWriter};
use smoltcp::{phy, time::Instant};

use crate::{buffer::RxBuffer, AnyNetworkDevice, TxBuffer};

impl phy::Device for dyn AnyNetworkDevice {
    type RxToken<'a> = RxToken;
    type TxToken<'a> = TxToken<'a>;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let tx_buffers = self.free_processed_tx_buffers();
        if self.can_receive() && self.can_send() {
            let rx_buffer = self.receive().unwrap();
            let tx_token = TxToken {
                device: self,
                tx_buffers,
            };
            Some((RxToken(rx_buffer), tx_token))
        } else {
            None
        }
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        let tx_buffers = self.free_processed_tx_buffers();

        if self.can_send() {
            let tx_token = TxToken {
                device: self,
                tx_buffers,
            };
            Some(tx_token)
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
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut packet = self.0.packet();
        let mut buffer = vec![0u8; packet.remain()];
        packet.read(&mut VmWriter::from(&mut buffer as &mut [u8]));
        f(&mut buffer)
    }
}

pub struct TxToken<'a> {
    device: &'a mut dyn AnyNetworkDevice,
    tx_buffers: Vec<TxBuffer>,
}

impl<'a> phy::TxToken for TxToken<'a> {
    fn consume<R, F>(mut self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buffer = vec![0u8; len];
        let res = f(&mut buffer);

        let mut reader = VmReader::from(buffer.as_slice());
        let mut tx_buffers = VecDeque::new();

        while reader.has_remain() {
            let mut tx_buffer = if let Some(tx_buffer) = self.tx_buffers.pop() {
                tx_buffer
            } else {
                break;
            };

            tx_buffer.set_packet(&mut reader);
            tx_buffers.push_back(tx_buffer);
        }

        if !tx_buffers.is_empty() {
            self.device
                .send_buffers(tx_buffers)
                .expect("fails to send buffers");
        }

        if reader.has_remain() {
            self.device.send(&mut reader).expect("fails to send packet");
        }

        res
    }
}
