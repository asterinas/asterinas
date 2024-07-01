// SPDX-License-Identifier: MPL-2.0

use core::fmt::Debug;

use ostd::Pod;

#[derive(Clone, Copy, Pod)]
#[repr(C)]
pub struct VirtioMmioLayout {
    /// Magic value: 0x74726976. **Read-only**
    pub magic_value: u32,
    /// Device version. 1 => Legacy, 2 => Normal. **Read-only**
    pub version: u32,
    /// Virtio Subsystem Device ID. **Read-only**
    pub device_id: u32,
    /// Virtio Subsystem Vendor ID. **Read-only**
    pub vendor_id: u32,

    /// Flags representing features the device supports.
    /// Bits 0-31 if `device_features_sel`is 0,
    /// bits 32-63 if `device_features_sel` is 1.
    /// **Read-only**
    pub device_features: u32,
    /// **Write-only**
    pub device_features_select: u32,

    __r1: [u8; 8],

    /// Flags representing device features understood and activated by the driver.
    /// Bits 0-31 if `driver_features_sel`is 0,
    /// bits 32-63 if `driver_features_sel` is 1.
    /// **Write-only**
    pub driver_features: u32,
    /// **Write-only**
    pub driver_features_select: u32,

    /// Guest page size.
    ///
    /// The driver writes the guest page size in bytes
    /// to the register during initialization, before any queues are used.
    ///
    /// This value should be a power of 2 and is used by the device to
    /// calculate the Guest address of the first queue page (see legacy_queue_pfn).
    /// **Write-only**
    pub legacy_guest_page_size: u32,

    __r2: [u8; 4],

    /// Selected queue. **Write-only**
    pub queue_sel: u32,
    /// Maximum virtual queue size. **Read-only**
    pub queue_num_max: u32,
    /// Virtual queue size. **Write-only**
    pub queue_num: u32,

    pub legacy_queue_align: u32,

    pub legacy_queue_pfn: u32,

    /// Virtual queue ready bit.
    ///
    /// Write 1 to notifies the device that it can execute requests from this virtual queue.
    /// **Read-Write**
    pub queue_ready: u32,

    __r3: [u8; 8],

    /// Queue notifier.
    ///
    /// Writing a value to this register notifies the device
    /// that there are new buffers to process in a queue. **Write-only**
    pub queue_notify: u32,

    __r4: [u8; 12],

    /// Interrupt status.
    ///
    /// bit0 => Used Buffer Notification;
    /// bit1 => Configuration Change Notification
    /// **Read-only**
    pub interrupt_status: u32,
    /// Interrupt acknowledge. **Write-only**
    pub interrupt_ack: u32,

    __r5: [u8; 8],

    /// Device status. **Read-Write**
    pub status: u32,

    __r6: [u8; 12],

    /// Virtual queue’s Descriptor Area 64 bit long physical address. **Write-only**
    pub queue_desc_low: u32,
    pub queue_desc_high: u32,

    __r7: [u8; 8],

    /// Virtual queue’s Driver Area 64 bit long physical address. **Write-only**
    pub queue_driver_low: u32,
    pub queue_driver_high: u32,

    __r8: [u8; 8],

    /// Virtual queue’s Device Area 64 bit long physical address. **Write-only**
    pub queue_device_low: u32,
    pub queue_device_high: u32,

    __r9: [u8; 84],

    /// Configuration atomicity value. **Read-only**
    pub config_generation: u32,
}

impl Debug for VirtioMmioLayout {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VirtioMmioLayout")
            .field("magic_value", &self.magic_value)
            .field("version", &self.version)
            .field("device_id", &self.device_id)
            .field("vendor_id", &self.vendor_id)
            .field("device_features", &self.device_features)
            .field("device_features_sel", &self.device_features_select)
            .field("driver_features", &self.driver_features)
            .field("driver_features_sel", &self.driver_features_select)
            .field("legacy_guest_page_size", &self.legacy_guest_page_size)
            .field("queue_sel", &self.queue_sel)
            .field("queue_num_max", &self.queue_num_max)
            .field("queue_num", &self.queue_num)
            .field("legacy_queue_align", &self.legacy_queue_align)
            .field("legacy_queue_pfn", &self.legacy_queue_pfn)
            .field("queue_ready", &self.queue_ready)
            .field("queue_notify", &self.queue_notify)
            .field("interrupt_status", &self.interrupt_status)
            .field("interrupt_ack", &self.interrupt_ack)
            .field("status", &self.status)
            .field("queue_desc_low", &self.queue_desc_low)
            .field("queue_desc_high", &self.queue_desc_high)
            .field("queue_driver_low", &self.queue_driver_low)
            .field("queue_driver_high", &self.queue_driver_high)
            .field("queue_device_low", &self.queue_device_low)
            .field("queue_device_high", &self.queue_device_high)
            .field("config_generation", &self.config_generation)
            .finish()
    }
}
