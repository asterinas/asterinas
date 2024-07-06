// SPDX-License-Identifier: MPL-2.0

//! CPU-related definitions.

pub mod cpu_local;

cfg_if::cfg_if! {
    if #[cfg(target_arch = "x86_64")]{
        pub use crate::arch::x86::cpu::*;
    }
}

use alloc::vec::Vec;

use bitvec::{
    prelude::{BitVec, Lsb0},
    slice::IterOnes,
};
use spin::Once;

use crate::{arch::boot::smp::get_processor_info, cpu};

/// The number of CPUs.
pub static NUM_CPUS: Once<u32> = Once::new();

/// Initializes the number of CPUs.
pub fn init() {
    let processor_info = get_processor_info();
    let num_processors = match processor_info {
        Some(info) => info.application_processors.len() + 1,
        None => 1,
    };
    NUM_CPUS.call_once(|| num_processors as u32);
}

/// Returns the number of CPUs.
pub fn num_cpus() -> u32 {
    *NUM_CPUS.get().unwrap()
}

/// Returns the ID of this CPU.
///
/// The CPU ID is strategically placed at the beginning of the CPU local storage area.
pub fn this_cpu() -> u32 {
    // SAFETY: the cpu ID is stored at the beginning of the cpu local area, provided
    // by the linker script.
    unsafe { (cpu::local::get_base() as usize as *mut u32).read() }
}

/// A subset of all CPUs in the system.
///
/// This structure can be used to mask out a subset of CPUs in the system.
#[derive(Clone, Debug, Default)]
pub struct CpuSet {
    bitset: BitVec,
}

impl CpuSet {
    /// Creates a new `CpuSet` with all CPUs in the system.
    pub fn new_full() -> Self {
        let num_cpus = num_cpus();
        let mut bitset = BitVec::with_capacity(num_cpus as usize);
        bitset.resize(num_cpus as usize, true);
        Self { bitset }
    }

    /// Creates a new `CpuSet` with no CPUs in the system.
    pub fn new_empty() -> Self {
        let num_cpus = num_cpus();
        let mut bitset = BitVec::with_capacity(num_cpus as usize);
        bitset.resize(num_cpus as usize, false);
        Self { bitset }
    }

    /// Adds a CPU to the set.
    pub fn add(&mut self, cpu_id: u32) {
        self.bitset.set(cpu_id as usize, true);
    }

    /// Adds a list of CPUs to the set.
    pub fn add_from_vec(&mut self, cpu_ids: Vec<u32>) {
        for cpu_id in cpu_ids {
            self.add(cpu_id)
        }
    }

    /// Adds all CPUs to the set.
    pub fn add_all(&mut self) {
        self.bitset.fill(true);
    }

    /// Removes a CPU from the set.
    pub fn remove(&mut self, cpu_id: u32) {
        self.bitset.set(cpu_id as usize, false);
    }

    /// Removes a list of CPUs from the set.
    pub fn remove_from_vec(&mut self, cpu_ids: Vec<u32>) {
        for cpu_id in cpu_ids {
            self.remove(cpu_id);
        }
    }

    /// Removes all CPUs from the set.
    pub fn clear(&mut self) {
        self.bitset.fill(false);
    }

    /// Returns true if the set contains the specified CPU.
    pub fn contains(&self, cpu_id: u32) -> bool {
        self.bitset.get(cpu_id as usize).as_deref() == Some(&true)
    }

    /// Iterates over the CPUs in the set.
    pub fn iter(&self) -> IterOnes<'_, usize, Lsb0> {
        self.bitset.iter_ones()
    }
}
