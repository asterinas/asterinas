pub mod virtio;
use jinux_frame::info;

pub fn init() {
    jinux_pci::init();
    for index in 0..jinux_pci::device_amount() {
        let pci_device = jinux_pci::get_pci_devices(index)
            .expect("initialize pci device failed: pci device is None");
        if pci_device.id.vendor_id == 0x1af4
            && (pci_device.id.device_id == 0x1001 || pci_device.id.device_id == 0x1042)
        {
            info!("found virtio block device");
            virtio::block::init(pci_device);
        }
    }
    info!("pci initialization complete")
}
