// SPDX-License-Identifier: MPL-2.0

//! RISC-V Platform Level Interrupt Controller

use alloc::collections::vec_deque::Iter;

use spin::Once;

use crate::{
    arch::{boot::DEVICE_TREE, device::io_port::PortRead},
    bus::pci::capability,
    io::{IoMem, IoMemAllocatorBuilder},
    mm::{paddr_to_vaddr, CachePolicy, Paddr, PageFlags, VmIoOnce},
    trap::TrapFrame,
};

/// RISC-V Platform Level Interrupt Controller.
pub struct PLIC {
    io_mem: IoMem,
}

impl PLIC {
    const SOURCE_CONUT: usize = 1024;
    const CONTEXT_COUNT: usize = 15872;
    const U32_BITS: usize = u32::BITS as _;
    const U32_BYTES: usize = u32::BITS as usize / 8;

    /// Create a new PLIC instance.
    ///
    /// # Arguments
    /// * `base_paddr` - The base physical address of the PLIC.
    /// * `size` - The size of the PLIC in bytes.
    pub fn new(base_paddr: Paddr, size: usize) -> Self {
        // SAFETY: We are building I/O memory using a region that is
        // specified as PLIC I/O memory in device tree.
        let io_mem = unsafe {
            IoMem::new(
                base_paddr..base_paddr + size,
                PageFlags::RW,
                CachePolicy::Uncacheable,
            )
        };
        Self { io_mem }
    }

    /// Base address of the PLIC.
    pub fn address(&self) -> Paddr {
        self.io_mem.paddr()
    }

    /// Grants access to the MMIO of the PLIC.
    pub fn io_mem(&self) -> &IoMem {
        &self.io_mem
    }

    /// Set the priority of a specific IRQ.
    ///
    /// The provided priority must be between 0 and 7, inclusive.
    /// If the priority is set to 0, the IRQ will be disabled.
    /// If the priority is set to 1, it will be the lowest active priority.
    /// If the priority is set to 7, it will be the highest priority.
    pub fn set_priority(&self, interrupt: u8, priority: u32) {
        assert!(priority <= 7, "Priority must be between 0 and 7");
        let offset = interrupt as usize * Self::U32_BYTES;
        self.io_mem.write_once::<u32>(offset, &priority).unwrap();
    }

    /// Get the priority of a specific IRQ.
    pub fn get_priority(&self, interrupt: u8) -> u32 {
        let offset = interrupt as usize * Self::U32_BYTES;
        self.io_mem.read_once::<u32>(offset).unwrap()
    }

    /// Check if a specific IRQ is pending,
    /// which means the interrupt is active and needs to be handled or is being handled.
    pub fn is_pending(&self, interrupt: u8) -> bool {
        let block = interrupt as usize / Self::U32_BITS;
        let index = interrupt as usize % Self::U32_BITS;
        let offset = 0x1000 + block * Self::U32_BYTES;
        let value = self.io_mem.read_once::<u32>(offset).unwrap();
        (value & (1 << index)) != 0
    }

    /// Enable a specific IRQ.
    pub fn enable(&self, hart_id: usize, is_s_mode: bool, interrupt: u8) {
        let context_id = Self::get_context_id(hart_id, is_s_mode);
        let pos = context_id * Self::SOURCE_CONUT + interrupt as usize;
        let block = pos / Self::U32_BITS;
        let index = pos % Self::U32_BITS;
        let offset = 0x2000 + block * Self::U32_BYTES;
        let old_value = self.io_mem.read_once::<u32>(offset).unwrap();
        let new_value = old_value | (1 << index);
        self.io_mem.write_once::<u32>(offset, &new_value).unwrap();
    }

    /// Disable a specific IRQ.
    pub fn disable(&self, hart_id: usize, is_s_mode: bool, interrupt: u8) {
        let context_id = Self::get_context_id(hart_id, is_s_mode);
        let pos = context_id * Self::SOURCE_CONUT + interrupt as usize;
        let block = pos / Self::U32_BITS;
        let index = pos % Self::U32_BITS;
        let offset = 0x2000 + block * Self::U32_BYTES;
        let old_value = self.io_mem.read_once::<u32>(offset).unwrap();
        let new_value = old_value & !(1 << index);
        self.io_mem.write_once::<u32>(offset, &new_value).unwrap();
    }

    /// Check if a specific IRQ is enabled.
    pub fn is_enabled(&self, hart_id: usize, is_s_mode: bool, interrupt: u8) -> bool {
        let context_id = Self::get_context_id(hart_id, is_s_mode);
        let pos = context_id * Self::SOURCE_CONUT + interrupt as usize;
        let block = pos / Self::U32_BITS;
        let index = pos % Self::U32_BITS;
        let offset = 0x2000 + block * Self::U32_BYTES;
        let value = self.io_mem.read_once::<u32>(offset).unwrap();
        (value & (1 << index)) != 0
    }

    /// Set the threshold for a specific hart.
    /// The provided threshold must be between 0 and 7, inclusive.
    /// If the threshold is set to 0, all IRQs will be enabled.
    pub fn set_threshold(&self, hart_id: usize, is_s_mode: bool, threshold: u32) {
        assert!(threshold <= 7, "Threshold must be between 0 and 7");
        let context_id = Self::get_context_id(hart_id, is_s_mode);
        let offset = 0x200000 + context_id * 4096;
        self.io_mem.write_once::<u32>(offset, &threshold).unwrap();
    }

    /// Get the threshold for a specific hart.
    ///
    /// Return a tuple of (Machine Mode Threshold, Supervisor Mode Threshold).
    pub fn get_threshold(&self, hart_id: usize, is_s_mode: bool) -> u32 {
        let context_id = Self::get_context_id(hart_id, is_s_mode);
        let offset = 0x200000 + context_id * 4096;
        let threshold = self.io_mem.read_once::<u32>(offset).unwrap();
        assert!(threshold <= 7, "Threshold must be between 0 and 7");
        threshold
    }

    /// Claim an interrupt.
    ///
    /// Return the active interrupt number if there is an active interrupt, otherwise return None.
    pub fn claim(&self, hart_id: usize, is_s_mode: bool) -> Option<u32> {
        let context_id = Self::get_context_id(hart_id, is_s_mode);
        let offset = 0x200004 + context_id * 4096;
        let value = self.io_mem.read_once::<u32>(offset).unwrap();
        if value == 0 {
            None
        } else {
            Some(value)
        }
    }

    /// Complete an interrupt.
    pub fn complete(&self, hart_id: usize, is_s_mode: bool, interrupt: u8) {
        let context_id = Self::get_context_id(hart_id, is_s_mode);
        let offset = 0x200004 + context_id * 4096;
        self.io_mem
            .write_once::<u32>(offset, &(interrupt as u32))
            .unwrap();
    }

    fn get_context_id(hart_id: usize, is_s_mode: bool) -> usize {
        if is_s_mode {
            hart_id + 1
        } else {
            hart_id
        }
    }
}

pub static PLIC: Once<PLIC> = Once::new();

pub fn init(io_mem_builder: &IoMemAllocatorBuilder) {
    let node = DEVICE_TREE.get().unwrap().find_node("/soc/plic").unwrap();
    if let Some(compatible) = node.compatible()
        && compatible
            .all()
            .any(|c| c == "sifive,plic-1.0.0" || c == "riscv,plic0")
    {
        let region = node.reg().unwrap().next().unwrap();
        PLIC.call_once(|| PLIC::new(region.starting_address as usize, region.size.unwrap()));
        io_mem_builder.remove(
            region.starting_address as usize
                ..region.starting_address as usize + region.size.unwrap(),
        );

        // SAFETY: Now we can start the external interrupts.
        unsafe { riscv::register::sie::set_sext() };
    }
}

pub(crate) fn claim_interrupt(hart_id: usize) -> usize {
    match PLIC.get().unwrap().claim(hart_id, true) {
        Some(interrupt) => interrupt as usize,
        None => 0,
    }
}

pub(crate) fn complete_interrupt(hart_id: usize, interrupt_source: usize) {
    PLIC.get()
        .unwrap()
        .complete(hart_id, true, interrupt_source as u8);
}
