//! The std library of kxos
#![no_std]
#![forbid(unsafe_code)]
#![allow(dead_code)]
#![allow(unused_variables)]
#![feature(const_btree_new)]
#![feature(cstr_from_bytes_until_nul)]

use kxos_frame::println;
use process::Process;

extern crate alloc;

mod memory;
mod process;
mod syscall;
mod util;

pub fn init() {
    process::fifo_scheduler::init();
}

pub fn run_first_kernel_task() {
    Process::spawn_kernel_task(|| {
        println!("hello world from kernel");
    });
}

pub fn run_first_process() {
    let elf_file_content = read_elf_content();
    Process::spawn_from_elf(elf_file_content);
}

fn read_elf_content() -> &'static [u8] {
    include_bytes!("../../kxos-user/hello_world/hello_world")
}
