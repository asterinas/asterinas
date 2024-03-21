// SPDX-License-Identifier: MPL-2.0

use aster_frame::cpu::{this_cpu, CpuSet};

use crate::{
    current_thread,
    thread::{
        kernel_thread::{KernelThreadExt, ThreadOptions},
        Thread,
    },
};

pub fn init() {
    aster_frame::smp::boot_all_aps();
    aster_frame::smp::register_ap_late_entry(ap_entry);
}

pub fn ap_entry() -> ! {
    run_ap_first_process();
}

fn run_ap_first_process() -> ! {
    let cpu_id = this_cpu();
    let mut cpu_set = CpuSet::new_empty();
    cpu_set.add(cpu_id);
    assert!(cpu_id != 0);
    Thread::spawn_kernel_thread(ThreadOptions::new(ap_thread).cpu_affinity(cpu_set));
    unreachable!()
}

fn ap_thread() {
    info!(
        "[kernel] Spawn init thread for processor {}, tid = {}",
        this_cpu(),
        current_thread!().tid()
    );
    loop {
        Thread::yield_now();
    }
}
