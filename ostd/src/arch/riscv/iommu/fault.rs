// SPDX-License-Identifier: MPL-2.0

//! Fault queue management for the RISC-V IOMMU.

use spin::Once;

use super::{queue::Queue, registers};
use crate::{error, mm::PAGE_SIZE, sync::SpinLock, warn};

/// Fault queue entry size in bytes.
const FAULT_ENTRY_SIZE: usize = 32;

/// Initializes the fault queue and enables fault reporting.
pub(super) fn init() {
    let mut iommu_regs = registers::IOMMU_REGS.get().unwrap().lock();

    let fq: Queue<32> = Queue::new();
    let fq_base = registers::QueueBase::new((fq.base_paddr() >> 12) as u64, fq.log2sz_minus_1());
    iommu_regs.fqb.as_mut_ptr().write(fq_base.value());
    iommu_regs.fqh.as_mut_ptr().write(0);

    // TODO: Enable the fault queue interrupt (FIE) once the interrupt
    // handler is wired. FIE is bit 1 in the same register as FQEN.
    iommu_regs.fqcsr.as_mut_ptr().write(registers::FQCSR_FQEN);

    // FQON should be asserted synchronously after FQEN is written;
    // give the hardware one pause, then bail if it isn't ready.
    core::hint::spin_loop();
    if iommu_regs.fqcsr.as_ptr().read() & registers::FQCSR_FQON == 0 {
        warn!("Fault queue did not become operational, disabling fault reporting");
        return;
    }

    FAULT_QUEUE.call_once(|| SpinLock::new(fq));
}

/// Drains and logs all pending fault records from the fault queue.
// TODO: Wire this as the IOMMU fault interrupt handler once the interrupt
// subsystem supports RISC-V IOMMU fault interrupts.
pub(super) fn process_faults() {
    let mut iommu_regs = registers::IOMMU_REGS.get().unwrap().lock();

    let fqcsr = iommu_regs.fqcsr.as_ptr().read();
    if fqcsr & (registers::FQCSR_FQMF | registers::FQCSR_FQOF) != 0 {
        error!("IOMMU fault queue error: fqcsr=0x{:x}", fqcsr);
        // Clears error bits (write-1-to-clear) to re-enable fault reporting.
        iommu_regs
            .fqcsr
            .as_mut_ptr()
            .write(fqcsr & (registers::FQCSR_FQMF | registers::FQCSR_FQOF));
    }

    iommu_regs.ipsr.as_mut_ptr().write(registers::IPSR_FIP);

    let fqt = iommu_regs.fqt.as_ptr().read() as usize;
    let fqh_val = iommu_regs.fqh.as_ptr().read() as usize;

    if fqt == fqh_val {
        return;
    }

    let num_records = if fqt > fqh_val {
        fqt - fqh_val
    } else {
        // Wrap-around case: fqt wrapped past the queue end.
        (PAGE_SIZE / FAULT_ENTRY_SIZE) - fqh_val + fqt
    };

    drop(iommu_regs);

    // TODO: Parse individual 32-byte fault records instead of just logging
    // the count.
    error!(
        "IOMMU fault: {} record(s) pending (fqh={}, fqt={})",
        num_records, fqh_val, fqt
    );

    let mut iommu_regs = registers::IOMMU_REGS.get().unwrap().lock();
    iommu_regs.fqh.as_mut_ptr().write(fqt as u32);
}

/// Fault queue singleton, initialized by [`init`] during IOMMU setup.
static FAULT_QUEUE: Once<SpinLock<Queue<32>>> = Once::new();
