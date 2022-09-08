//! The std library of kxos
#![no_std]
#![forbid(unsafe_code)]
#![allow(dead_code)]
#![allow(unused_variables)]
#![feature(const_btree_new)]
#![feature(cstr_from_bytes_until_nul)]

use kxos_frame::{info, println, task::Task};
use process::Process;

extern crate alloc;

mod memory;
mod process;
mod syscall;
mod util;

pub fn init() {
    process::fifo_scheduler::init();
}

pub fn init_task() {
    println!("[kernel] Hello world from init task!");

    let process = Process::spawn_kernel_task(|| {
        println!("[kernel] Hello world from kernel!");
    });
    info!("spawn kernel process, pid = {}", process.pid());

    let elf_file_content = read_elf_content();
    let process = Process::spawn_from_elf(elf_file_content);
    info!("spwan user process, pid = {}", process.pid());

    loop {}
}

/// first process never return
pub fn run_first_process() -> ! {
    let elf_file_content = read_elf_content();
    Task::spawn(init_task, None::<u8>, None).expect("Spawn first task failed");
    unreachable!()
}

fn read_elf_content() -> &'static [u8] {
    include_bytes!("../../kxos-user/hello_world/hello_world")
}
