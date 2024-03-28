// SPDX-License-Identifier: MPL-2.0

use aster_frame::{
    cpu::{this_cpu, CpuSet},
    prelude::aster_ap_entry,
};
use log::info;

use crate::{
    current_thread, println,
    sched::init_local_scheduler,
    thread::{
        kernel_thread::{KernelThreadExt, ThreadOptions},
        Thread,
    },
};

pub fn init() {
    aster_frame::smp::boot_all_aps();
}

#[aster_ap_entry]
pub fn ap_entry() -> ! {
    run_ap_first_process();
}

fn run_ap_first_process() -> ! {
    init_local_scheduler();
    let cpu_id = this_cpu();
    info!("hello from cpu {}", cpu_id);
    assert!(cpu_id != 0);
    Thread::spawn_kernel_thread(
        ThreadOptions::new(ap_thread).cpu_affinity(CpuSet::from_cpu_id(cpu_id)),
    );
    unreachable!()
}

fn ap_thread() {
    println!(
        "[kernel] Spawn init thread for processor {}, tid = {}",
        this_cpu(),
        current_thread!().tid()
    );
    loop {
        Thread::yield_now();
    }
}
