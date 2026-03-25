// SPDX-License-Identifier: MPL-2.0

//! The device object of virtio-vsock.

use alloc::{boxed::Box, string::ToString, sync::Arc};
use core::sync::atomic::{AtomicU64, Ordering};

use aster_softirq::BottomHalfDisabled;
use ostd::{
    arch::trap::TrapFrame,
    debug,
    sync::{SpinLock, SpinLockGuard},
};
use spin::Once;

use crate::{
    device::{
        VirtioDeviceError,
        socket::{
            DEVICE_NAME,
            config::{VirtioVsockConfig, VsockFeatures},
            header::VirtioVsockEventId,
            queue::{EventQueue, RxQueue, TxQueue},
        },
    },
    transport::{ConfigManager, VirtioTransport},
};

/// Socket devices, which facilitate data transfer between the guest and device without using the
/// Ethernet or IP protocols.
pub struct SocketDevice {
    config_manager: ConfigManager<VirtioVsockConfig>,
    guest_cid: AtomicU64,
    tx_queue: SpinLock<TxQueue, BottomHalfDisabled>,
    rx_queue: SpinLock<RxQueue, BottomHalfDisabled>,
    rx_callback: Once<fn()>,
    event_queue: SpinLock<EventQueue, BottomHalfDisabled>,
    event_callback: Once<fn()>,
    transport: SpinLock<Box<dyn VirtioTransport>>,
}

impl SocketDevice {
    /// Negotiates the subset of device features supported by this driver.
    pub(crate) fn negotiate_features(features: u64) -> u64 {
        (VsockFeatures::from_bits_truncate(features) & VsockFeatures::supported_features()).bits()
    }

    /// Initializes a virtio-vsock device from `transport` and registers it globally.
    pub(crate) fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let config_manager = VirtioVsockConfig::new_manager(transport.as_ref());
        let guest_cid = VirtioVsockConfig::read_guest_cid(&config_manager);

        let tx_queue = TxQueue::new(transport.as_mut())?;
        let rx_queue = RxQueue::new(transport.as_mut())?;
        let event_queue = EventQueue::new(transport.as_mut())?;

        let device = Arc::new(Self {
            config_manager,
            guest_cid: AtomicU64::new(guest_cid),
            tx_queue: SpinLock::new(tx_queue),
            rx_queue: SpinLock::new(rx_queue),
            rx_callback: Once::new(),
            event_queue: SpinLock::new(event_queue),
            event_callback: Once::new(),
            transport: SpinLock::new(transport),
        });

        let mut transport = device.transport.lock();
        let weak_device = Arc::downgrade(&device);
        transport
            .register_queue_callback(
                RxQueue::QUEUE_INDEX,
                Box::new(move |_: &TrapFrame| super::schedule_rx(&weak_device)),
                true,
            )
            .unwrap();
        let weak_device = Arc::downgrade(&device);
        transport
            .register_queue_callback(
                TxQueue::QUEUE_INDEX,
                Box::new(move |_: &TrapFrame| super::schedule_tx(&weak_device)),
                true,
            )
            .unwrap();
        let weak_device = Arc::downgrade(&device);
        transport
            .register_queue_callback(
                EventQueue::QUEUE_INDEX,
                Box::new(move |_: &TrapFrame| super::schedule_event(&weak_device)),
                true,
            )
            .unwrap();
        transport
            .register_cfg_callback(Box::new(config_space_change))
            .unwrap();
        transport.finish_init();
        drop(transport);

        // Reload the guest CID after initialization to prevent race conditions if the CID changes
        // at the same time.
        device.reload_guest_id();

        super::register_device(DEVICE_NAME.to_string(), device);
        Ok(())
    }

    /// Locks the transmit queue.
    pub fn lock_tx(&self) -> SpinLockGuard<'_, TxQueue, BottomHalfDisabled> {
        self.tx_queue.lock()
    }

    /// Locks the receive queue.
    pub fn lock_rx(&self) -> SpinLockGuard<'_, RxQueue, BottomHalfDisabled> {
        self.rx_queue.lock()
    }

    /// Returns the current guest CID reported by the device configuration space.
    pub fn guest_cid(&self) -> u64 {
        self.guest_cid.load(Ordering::Relaxed)
    }

    /// Registers the callback invoked after a packet is received.
    ///
    /// The function may be called only once; subsequent calls take no effect.
    pub fn init_rx_callback(&self, callback: fn()) {
        self.rx_callback.call_once(|| callback);
    }

    pub(super) fn process_rx(&self) {
        if let Some(callback) = self.rx_callback.get() {
            (callback)();
        }
    }

    /// Registers the callback invoked after a transport event is received.
    ///
    /// The function may be called only once; subsequent calls take no effect.
    pub fn init_event_callback(&self, callback: fn()) {
        self.event_callback.call_once(|| callback);
    }

    pub(super) fn process_event(&self) {
        let mut event_queue = self.event_queue.lock();

        let Some(event_id) = event_queue.recv() else {
            return;
        };
        match event_id {
            VirtioVsockEventId::TransportReset => (),
        }

        drop(event_queue);

        if let Some(callback) = self.event_callback.get() {
            (callback)();
        } else {
            // Reload the CID, even if the callback is not yet available. Otherwise, the callback is
            // expected to do so.
            self.reload_guest_id();
        }
    }

    /// Reloads the guest CID from the device configuration space.
    ///
    /// This is used after transport reset events, which may change the local CID.
    pub fn reload_guest_id(&self) {
        let guest_cid = VirtioVsockConfig::read_guest_cid(&self.config_manager);
        self.guest_cid.store(guest_cid, Ordering::Relaxed);
    }
}

fn config_space_change(_: &TrapFrame) {
    debug!("virtio-vsock config change");
}
