use alloc::sync::Arc;
use aster_frame::bus::mmio::MMIO_BUS;
use spin::Once;

use self::driver::VirtioMmioDriver;

pub mod device;
pub mod driver;
pub mod layout;
pub mod multiplex;

pub static VIRTIO_MMIO_DRIVER: Once<Arc<VirtioMmioDriver>> = Once::new();
pub fn virtio_mmio_init() {
    VIRTIO_MMIO_DRIVER.call_once(|| Arc::new(VirtioMmioDriver::new()));
    MMIO_BUS
        .lock()
        .register_driver(VIRTIO_MMIO_DRIVER.get().unwrap().clone());
}
