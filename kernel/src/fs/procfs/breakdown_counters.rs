// SPDX-License-Identifier: MPL-2.0

use alloc::format;
use core::sync::atomic::{AtomicU64, Ordering};

use ostd::{
    cpu::{all_cpus, CpuId},
    cpu_local, cpu_local_cell,
};

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
};

/// Represents the inode at `/proc/breakdown-counters`.
pub struct BreakdownCountersFileOps;

impl BreakdownCountersFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self).parent(parent).build().unwrap()
    }
}

impl FileOps for BreakdownCountersFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let mut user = 0;
        let mut mmap = 0;
        let mut page_fault = 0;
        let mut munmap = 0;
        let mut mprotect = 0;
        let mut idle = 0;
        let mut sched = 0;

        for cpu in all_cpus() {
            user += USER.get_on_cpu(cpu).load(Ordering::Relaxed);
            mmap += MMAP.get_on_cpu(cpu).load(Ordering::Relaxed);
            page_fault += PAGE_FAULT.get_on_cpu(cpu).load(Ordering::Relaxed);
            munmap += MUNMAP.get_on_cpu(cpu).load(Ordering::Relaxed);
            mprotect += MPROTECT.get_on_cpu(cpu).load(Ordering::Relaxed);
            idle += IDLE.get_on_cpu(cpu).load(Ordering::Relaxed);
            sched += SCHED.get_on_cpu(cpu).load(Ordering::Relaxed);
        }

        let cur = ostd::arch::read_tsc();

        let output = format!(
            "cur: {}\nuser: {}\nmmap: {}\npage_fault: {}\nmunmap: {}\nmprotect: {}\nidle: {}\nsched: {}\n",
            cur, user, mmap, page_fault, munmap, mprotect, idle, sched
        );

        Ok(output.into_bytes())
    }
}

cpu_local! {
    static USER: AtomicU64 = AtomicU64::new(0);
    static MMAP: AtomicU64 = AtomicU64::new(0);
    static PAGE_FAULT: AtomicU64 = AtomicU64::new(0);
    static MUNMAP: AtomicU64 = AtomicU64::new(0);
    static MPROTECT: AtomicU64 = AtomicU64::new(0);
    static IDLE: AtomicU64 = AtomicU64::new(0);
    static SCHED: AtomicU64 = AtomicU64::new(0);
}

cpu_local_cell! {
    static USER_START: u64 = 0;
    static MMAP_START: u64 = 0;
    static PAGE_FAULT_START: u64 = 0;
    static MUNMAP_START: u64 = 0;
    static MPROTECT_START: u64 = 0;
    static IDLE_START: u64 = 0;
    static SCHED_START: u64 = 0;
}

pub fn user_start() {
    USER_START.store(ostd::arch::read_tsc());
}

pub fn user_end() {
    let cpu = CpuId::current_racy();
    ();
    let cur = ostd::arch::read_tsc();
    USER.get_on_cpu(cpu)
        .fetch_add(cur - USER_START.load(), Ordering::Relaxed);
}

pub fn mmap_start() {
    MMAP_START.store(ostd::arch::read_tsc());
}

pub fn mmap_end() {
    let cpu = CpuId::current_racy();
    ();
    let cur = ostd::arch::read_tsc();
    MMAP.get_on_cpu(cpu)
        .fetch_add(cur - MMAP_START.load(), Ordering::Relaxed);
}

pub fn page_fault_start() {
    PAGE_FAULT_START.store(ostd::arch::read_tsc());
}

pub fn page_fault_end() {
    let cpu = CpuId::current_racy();
    ();
    let cur = ostd::arch::read_tsc();
    PAGE_FAULT
        .get_on_cpu(cpu)
        .fetch_add(cur - PAGE_FAULT_START.load(), Ordering::Relaxed);
}

pub fn munmap_start() {
    MUNMAP_START.store(ostd::arch::read_tsc());
}

pub fn munmap_end() {
    let cpu = CpuId::current_racy();
    ();
    let cur = ostd::arch::read_tsc();
    MUNMAP
        .get_on_cpu(cpu)
        .fetch_add(cur - MUNMAP_START.load(), Ordering::Relaxed);
}

pub fn mprotect_start() {
    MPROTECT_START.store(ostd::arch::read_tsc());
}

pub fn mprotect_end() {
    let cpu = CpuId::current_racy();
    ();
    let cur = ostd::arch::read_tsc();
    MPROTECT
        .get_on_cpu(cpu)
        .fetch_add(cur - MPROTECT_START.load(), Ordering::Relaxed);
}

pub fn idle_start() {
    IDLE_START.store(ostd::arch::read_tsc());
}

pub fn idle_end() {
    let cpu = CpuId::current_racy();
    ();
    let cur = ostd::arch::read_tsc();
    IDLE.get_on_cpu(cpu)
        .fetch_add(cur - IDLE_START.load(), Ordering::Relaxed);
}

pub fn sched_start() {
    SCHED_START.store(ostd::arch::read_tsc());
}

pub fn sched_end() {
    let cpu = CpuId::current_racy();
    ();
    let cur = ostd::arch::read_tsc();
    SCHED
        .get_on_cpu(cpu)
        .fetch_add(cur - SCHED_START.load(), Ordering::Relaxed);
}
