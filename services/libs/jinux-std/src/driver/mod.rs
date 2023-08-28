use log::info;

pub fn init() {
    // print all the input device to make sure input crate will compile
    for (name, _) in jinux_input::all_devices() {
        info!("Found Input device, name:{}", name);
    }
}

#[allow(unused)]
fn block_device_test() {
    for (_, device) in jinux_block::all_devices() {
        let mut write_buffer = [0u8; 512];
        let mut read_buffer = [0u8; 512];
        info!("write_buffer address:{:x}", write_buffer.as_ptr() as usize);
        info!("read_buffer address:{:x}", read_buffer.as_ptr() as usize);
        for i in 0..512 {
            for byte in write_buffer.iter_mut() {
                *byte = i as u8;
            }
            device.write_block(i as usize, &write_buffer);
            device.read_block(i as usize, &mut read_buffer);
            assert_eq!(write_buffer, read_buffer);
        }
        info!("block device test passed!");
    }
}
