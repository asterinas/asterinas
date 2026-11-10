// SPDX-License-Identifier: MPL-2.0

//!    Currently, the kernel's page tables only support VPNs with a uniform bit 
//!    width, whereas G-stage page tables require the root page table's VPN field 
//!    to be expanded to 11 bits; for instance, in Sv48*4 mode, the VPN structure 
//!    is [11, 9, 9, 9], rather than the uniform 9-bit segments found in the 
//!    standard Sv48 page tables currently used by the kernel.
//! 
//!    G-stage docs: https://docs.riscv.org/reference/isa/v20260120/priv/hypervisor.html#two-stage-translation
//!    issue: https://github.com/asterinas/asterinas/issues/4016
//! 
//!  ----------------------------------------------------------------------------
//! 
//!    NOTE:
//!        The code in this file currently does not comply with coding standards.
//! 
//! 
//!        This file provides only a minimal functional implementation.
//! 
//!        Once the kernel supports this feature, this file can be rewritten after 
//!        replacement and testing are complete.
//! 
//!    NOTE:
//!        Files should be checked if this file is going to be deleted:
//! 
//!        1.ostd/src/arch/riscv/vm/gstage.rs
//!        2.ostd/src/vm/mod.rs
//!        3.ostd/src/vm/gpm_space.rs
//!        4.ostd/src/vm/temp_riscv_gpm.rs
//!        5.kernel/src/kvm/vm_memory.rs(func map_guest_memory() & unmap_guest_memory())
//! 
//!  ****************************************************************************

use core::arch::asm;
use crate::prelude::*;
use crate::mm::*;

const PAGE_SIZE: usize = 4096;
pub(super) const PAGE_SHIFT: usize = 12;

const VPN_BITS: usize = 9;
const PPN_BITS: usize = 44;
const GSTAGE_LEVELS: usize = 4;

const PTE_V: u64 = 1 << 0;  // Valid
const PTE_R: u64 = 1 << 1;  // Read
const PTE_W: u64 = 1 << 2;  // Write
const PTE_X: u64 = 1 << 3;  // Execute
const PTE_U: u64 = 1 << 4;  // User
const PTE_G: u64 = 1 << 5;  // Global
const PTE_A: u64 = 1 << 6;  // Accessed
const PTE_D: u64 = 1 << 7;  // Dirty

const PTE_PPN_MASK: u64 = ((1u64 << PPN_BITS) - 1) << 10;
const PTE_ADDR_MASK: u64 = PTE_PPN_MASK | 0xFFF;

pub(crate) struct GStagePageTable {
    root: u64,
    root_paddr: u64,
    frames: Vec<Frame<()>>,
    seg: Vec<Segment<()>>,
}

impl GStagePageTable {
    pub(crate) fn new() -> Self {
        if let Ok(seg) = FrameAllocOptions::new().alloc_segment(4) {
            let root_paddr = seg.paddr() as u64;
            let root = paddr_to_vaddr(root_paddr as usize) as u64;
        
            unsafe {
                core::ptr::write_bytes(root as *mut u64, 0, 4 * PAGE_SIZE / 8);
            }

            let mut segv = Vec::new();
            segv.push(seg);
        
            Self { root_paddr,  root , frames: Vec::new(), seg: segv}
        } else {
            Self { root_paddr:0,  root:0, frames: Vec::new(), seg: Vec::new()}
        }
    }

    fn alloc_page(&mut self) -> (u64, u64) {
        if let Ok(frame) = FrameAllocOptions::new().alloc_frame() {
            let ret_va = paddr_to_vaddr(frame.paddr()) as u64;
            let ret_pa = frame.paddr() as u64;
            self.frames.push(frame);
            return (ret_va, ret_pa);
        }
        (0,0)
    }
    
    pub(crate) fn get_root(&self) -> u64 {
        self.root_paddr
    }

    pub(crate) fn map(
        &mut self,
        gpa: u64,
        hpa: u64,
        size: usize,
        flags: u64,
    ) -> Result<()> {
        let num_pages = size / PAGE_SIZE;
        let pte_flags = flags | PTE_V | PTE_A | PTE_D | PTE_X | PTE_U;
        
        for i in 0..num_pages {
            let offset = (i * PAGE_SIZE) as u64;
            self.map_page(gpa + offset, hpa + offset, pte_flags)?;
        }
        
        Ok(())
    }
    
    fn map_page(
        &mut self,
        gpa: u64,
        hpa: u64,
        flags: u64,
    ) -> Result<()> {
        let vpn = [
            (gpa >> (PAGE_SHIFT + 0 * VPN_BITS)) & 0x1FF,
            (gpa >> (PAGE_SHIFT + 1 * VPN_BITS)) & 0x1FF,
            (gpa >> (PAGE_SHIFT + 2 * VPN_BITS)) & 0x1FF,
            (gpa >> (PAGE_SHIFT + 3 * VPN_BITS)) & 0x7FF,
        ];
        
        let mut table = self.root as *mut u64;
        
        for level in (1..GSTAGE_LEVELS).rev() {
            let idx = vpn[level] as usize;
            let pte_ptr = unsafe { table.add(idx) };
            let pte = unsafe { core::ptr::read_volatile(pte_ptr) };
            
            if pte & PTE_V == 0 {
                let (new_table_va, new_table_pa) = self.alloc_page();
                let new_table = new_table_va as *mut u64;
                
                unsafe {
                    core::ptr::write_bytes(new_table, 0, PAGE_SIZE / 8);
                }
                
                let new_pte = ((new_table_pa as u64) >> PAGE_SHIFT << 10) | PTE_V;
                unsafe {
                    core::ptr::write_volatile(pte_ptr, new_pte);
                }
                
                table = new_table;
            } else {
                let next_table = ((pte & PTE_PPN_MASK) >> 10) << PAGE_SHIFT;
                table = paddr_to_vaddr(next_table as usize) as *mut u64;
            }
        }
        
        let idx = vpn[0] as usize;
        let pte_ptr = unsafe { table.add(idx) };
        let _pte = unsafe { core::ptr::read_volatile(pte_ptr) };
        
        let new_pte = ((hpa >> PAGE_SHIFT) << 10) | flags;
        unsafe {
            core::ptr::write_volatile(pte_ptr, new_pte);
        }
        
        Ok(())
    }
    
    pub(crate) fn unmap(&mut self, gpa: u64, size: usize) -> Result<usize> {
        let num_pages = size / PAGE_SIZE;
        
        for i in 0..num_pages {
            let offset = (i * PAGE_SIZE) as u64;
            let _ = self.unmap_page(gpa + offset);
        }
        
        Ok(num_pages)
    }
    
    fn unmap_page(&mut self, gpa: u64) -> Result<()> {
        let vpn = [
            (gpa >> (PAGE_SHIFT + 0 * VPN_BITS)) & 0x1FF,
            (gpa >> (PAGE_SHIFT + 1 * VPN_BITS)) & 0x1FF,
            (gpa >> (PAGE_SHIFT + 2 * VPN_BITS)) & 0x1FF,
            (gpa >> (PAGE_SHIFT + 3 * VPN_BITS)) & 0x7FF,
        ];
        
        let mut table = self.root as *mut u64;
        let mut tables_to_free = [core::ptr::null_mut(); GSTAGE_LEVELS - 1];
        let mut free_count = 0;
        
        for level in (1..GSTAGE_LEVELS).rev() {
            let idx = vpn[level] as usize;
            let pte_ptr = unsafe { table.add(idx) };
            let pte = unsafe { core::ptr::read_volatile(pte_ptr) };
                        
            let next_table = ((pte & PTE_PPN_MASK) >> 10) << PAGE_SHIFT;
            tables_to_free[free_count] = table;
            free_count += 1;
            table = next_table as *mut u64;
        }
        
        let idx = vpn[0] as usize;
        let pte_ptr = unsafe { table.add(idx) };
                
        unsafe {
            core::ptr::write_volatile(pte_ptr, 0);
            asm!("hfence.gvma");
        }
        Ok(())
    }
}
