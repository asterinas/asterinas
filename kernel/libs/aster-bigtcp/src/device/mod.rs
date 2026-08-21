// SPDX-License-Identifier: MPL-2.0

//! Network device abstractions.

use core::{any::Any, fmt::Debug};

use smoltcp::{
    iface::{Config, Interface},
    phy::Device,
};
pub use smoltcp::{
    phy::{Checksum, ChecksumCapabilities, DeviceCapabilities, Medium},
    wire::EthernetAddress,
};

use crate::{
    packet::{AllocatedTxPacket, LinkLayer, RxPacket, TxPacket},
    time::Instant,
};

mod loopback;
pub use loopback::Loopback;

#[derive(Clone, Copy, Debug)]
pub enum NetError {
    NotReady,
    Busy,
    NoMemory,
}

pub trait AnyNetworkDevice: Send + Sync + Any + Debug {
    // ================ Device Information =================

    fn mac_addr(&self) -> EthernetAddress;
    fn capabilities(&self) -> DeviceCapabilities;

    // ================ Device Operation ===================

    fn can_receive(&self) -> bool;
    fn can_send(&self) -> bool;

    /// Receives a link-layer packet from the network.
    ///
    /// If no packet is ready, this method returns [`NetError::NotReady`].
    fn receive(&mut self) -> Result<RxPacket<LinkLayer>, NetError>;

    /// Sends a link-layer packet to the network.
    ///
    /// It is recommended that the packet is allocated using [`Self::alloc_tx_buffer`] to improve
    /// performance (e.g., by reducing DMA overhead via a pool). However, this is not a strict
    /// requirement, and the method must work regardless of how the packet is allocated.
    fn send(&mut self, packet: TxPacket<LinkLayer>) -> Result<(), NetError>;

    /// Allocates a packet buffer for transmission through this device.
    fn alloc_tx_buffer(payload_len: usize) -> Result<AllocatedTxPacket, ostd::Error>
    where
        Self: Sized;

    /// Frees processed transmit buffers.
    fn free_processed_tx_buffers(&mut self);

    /// Notifies the device driver that a polling operation has ended.
    ///
    /// The driver can assume that the device remains protected by a poll lock for the entire
    /// duration of polling, so two polling operations cannot happen simultaneously.
    fn notify_poll_end(&mut self);
}

/// A trait that allows obtaining a mutable reference to an [`AnyNetworkDevice`].
///
/// A device is usually protected by a lock and may be stored behind a shared pointer. This trait
/// hides those storage details from the network interface.
pub trait WithDevice: Send + Sync {
    // This is a hack to make `AnyNetworkDevice::alloc_tx_buffer` usable without always acquiring
    // the device lock.
    type Device: AnyNetworkDevice;

    fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut dyn AnyNetworkDevice) -> R;
}

/// Creates a smoltcp [`Interface`] with the specified capabilities.
pub(crate) fn new_interface(
    config: Config,
    capabilities: DeviceCapabilities,
    now: Instant,
) -> Interface {
    let mut device = CapabilitiesDevice(capabilities);
    Interface::new(config, &mut device, now)
}

struct CapabilitiesDevice(DeviceCapabilities);

impl Device for CapabilitiesDevice {
    // The purpose of this device implementation is only to provide the device capability to satisfy
    // the smoltcp API (`Interface::new`). These tokens are placeholders that are never constructed.
    type RxToken<'a> = <smoltcp::phy::Loopback as Device>::RxToken<'a>;
    type TxToken<'a> = <smoltcp::phy::Loopback as Device>::TxToken<'a>;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        None
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        None
    }

    fn capabilities(&self) -> DeviceCapabilities {
        self.0.clone()
    }
}
