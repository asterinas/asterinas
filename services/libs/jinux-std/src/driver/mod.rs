use jinux_input::INPUT_COMPONENT;
use log::info;

pub fn init() {
    // print all the input device to make sure input crate will compile
    for comp in INPUT_COMPONENT.get().unwrap().get_input_device() {
        info!("input device name:{}", comp.name());
    }
}

#[allow(unused)]
fn block_device_test() {
    let block_device = jinux_block::BLK_COMPONENT.get().unwrap().get_device();
    let mut write_buffer = [0u8; 512];
    let mut read_buffer = [0u8; 512];
    info!("write_buffer address:{:x}", write_buffer.as_ptr() as usize);
    info!("read_buffer address:{:x}", read_buffer.as_ptr() as usize);
    for i in 0..512 {
        for byte in write_buffer.iter_mut() {
            *byte = i as u8;
        }
        block_device.write_block(i as usize, &write_buffer);
        block_device.read_block(i as usize, &mut read_buffer);
        assert_eq!(write_buffer, read_buffer);
    }
    info!("block device test passed!");
}
