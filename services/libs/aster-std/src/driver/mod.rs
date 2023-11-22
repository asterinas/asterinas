use core::mem::size_of;

use alloc::vec::Vec;
use aster_frame::{
    println,
    vm::{VmAllocOptions, VmIo},
};
use log::info;

pub fn init() {
    // print all the input device to make sure input crate will compile
    for (name, _) in aster_input::all_devices() {
        info!("Found Input device, name:{}", name);
    }
}

#[allow(unused)]
fn block_device_test() {
    for (_, device) in aster_block::all_devices() {
        let write_frame = VmAllocOptions::new(1).alloc_single().unwrap();
        let read_frame = VmAllocOptions::new(1).alloc_single().unwrap();
        info!("write_buffer address:{:x}", write_frame.start_paddr());
        info!("read_buffer address:{:x}", read_frame.start_paddr());

        // init write frame
        for i in 0..=8 {
            let slice: [u8; 512] = [i; 512];
            write_frame.write_slice(i as usize * 512, &slice);
        }

        // Test multiple Writer & Reader
        let mut writers = Vec::with_capacity(8);
        for i in 0..8 {
            let writer = read_frame.writer().skip(i * 512).limit(512);
            writers.push(writer);
        }

        let mut readers = Vec::with_capacity(8);
        for i in 0..8 {
            let reader = write_frame.reader().skip(i * 512).limit(512);
            readers.push(reader);
        }

        device.write_block(0, readers.as_slice());
        device.read_block(0, writers.as_slice());
        let mut read_slice = [0u8; 512];
        let mut write_slice = [0u8; 512];
        for i in 0..8 {
            read_frame.read_bytes(i * size_of::<[u8; 512]>(), &mut read_slice);
            write_frame.read_bytes(i * size_of::<[u8; 512]>(), &mut write_slice);
            assert_eq!(read_slice, write_slice);
        }
        println!("block device test passed!");
    }
}
