use crate::prelude::*;
use kxos_frame::vm::{Pod, VmIo};

pub mod vm_page;

use crate::process::Process;

/// copy bytes from user space of current process. The bytes len is the len of dest.
pub fn read_bytes_from_user(src: Vaddr, dest: &mut [u8]) {
    let current = Process::current();
    let vm_space = current
        .vm_space()
        .expect("[Internal error]Current should have vm space to copy bytes from user");
    vm_space.read_bytes(src, dest).expect("read bytes failed");
}

/// copy val (Plain of Data type) from user space of current process.
pub fn read_val_from_user<T: Pod>(src: Vaddr) -> T {
    let current = Process::current();
    let vm_space = current
        .vm_space()
        .expect("[Internal error]Current should have vm space to copy val from user");
    vm_space.read_val(src).expect("read val failed")
}

/// write bytes from user space of current process. The bytes len is the len of src.
pub fn write_bytes_to_user(dest: Vaddr, src: &[u8]) {
    let current = Process::current();
    let vm_space = current
        .vm_space()
        .expect("[Internal error]Current should have vm space to write bytes to user");
    vm_space.write_bytes(dest, src).expect("write bytes failed")
}

/// write val (Plain of Data type) to user space of current process.
pub fn write_val_to_user<T: Pod>(dest: Vaddr, val: &T) {
    let current = Process::current();
    let vm_space = current
        .vm_space()
        .expect("[Internal error]Current should have vm space to write val to user");
    vm_space.write_val(dest, val).expect("write val failed");
}
