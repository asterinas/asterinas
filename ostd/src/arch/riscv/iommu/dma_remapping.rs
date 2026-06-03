// SPDX-License-Identifier: MPL-2.0

//! DMA remapping for the RISC-V IOMMU.
//!
//! The initialization sequence in [`init`] follows spec Chapter 6 software
//! guidelines: command queue setup (Step 12), fault queue setup (Step 13),
//! DDT population and activation (Step 15). The DDT is activated last so the
//! IOMMU never sees partially-initialized state.

use spin::Once;

use super::{
    IommuError,
    ddt::DdtTable,
    queue::{self, Queue},
    registers,
    second_stage::IommuPtConfig,
};
use crate::{
    info,
    mm::{
        Daddr, PAGE_SIZE, PageProperty, PageTable,
        page_prop::{CachePolicy, PageFlags, PrivilegedPageFlags as PrivFlags},
    },
    prelude::Paddr,
    sync::{LocalIrqDisabled, SpinLock},
    task::disable_preempt,
    warn,
};

/// Returns `true` if DMA remapping has been initialized and is active.
pub fn has_dma_remapping() -> bool {
    PAGE_TABLE.get().is_some()
}

/// Maps a single page from a device address to a physical address.
///
/// # Safety
///
/// The physical address must point to untyped DMA memory that outlives this
/// mapping.
pub unsafe fn map(daddr: Daddr, paddr: Paddr) -> Result<(), IommuError> {
    let Some(table) = PAGE_TABLE.get() else {
        return Err(IommuError::NoIommu);
    };

    let locked_table = table.lock();
    let from = daddr..daddr + PAGE_SIZE;
    let prop = PageProperty {
        flags: PageFlags::RW,
        cache: CachePolicy::Uncacheable,
        priv_flags: PrivFlags::empty(),
    };

    let preempt_guard = disable_preempt();
    let mut cursor = locked_table
        .cursor_mut(&preempt_guard, &from)
        .map_err(IommuError::ModificationError)?;

    // SAFETY: The caller guarantees that paddr is valid untyped memory.
    unsafe { cursor.map((paddr, 1, prop)) };

    // TODO: Issue IOTLB invalidation (IOTINVAL.GVMA + IOFENCE.C) so the
    // IOMMU sees the new PTE. Deferred until hardware testing is possible;
    // QEMU's IOMMU does not cache translations, so this is not yet needed.

    Ok(())
}

/// Unmaps a single page at the given device address.
pub fn unmap(daddr: Daddr) -> Result<(), IommuError> {
    let Some(table) = PAGE_TABLE.get() else {
        return Err(IommuError::NoIommu);
    };

    let locked_table = table.lock();
    let preempt_guard = disable_preempt();
    let mut cursor = locked_table
        .cursor_mut(&preempt_guard, &(daddr..daddr + PAGE_SIZE))
        .map_err(IommuError::ModificationError)?;

    // SAFETY: Unmapping a page from the IOMMU page table is always safe;
    // it simply removes a translation that was previously established.
    let frag = unsafe { cursor.take_next(PAGE_SIZE) };
    debug_assert!(frag.is_some());

    // TODO: Issue IOTLB invalidation after PTE removal (same rationale as
    // in `map()`).

    Ok(())
}

/// Initializes DMA remapping.
pub fn init() {
    let caps = {
        let iommu_regs = registers::IOMMU_REGS.get().unwrap().lock();
        iommu_regs.capabilities
    };

    if !caps.flags().contains(registers::CapabilityFlags::SV39X4) {
        warn!("Sv39x4 second-stage translation not supported by IOMMU, disabling DMA remapping");
        return;
    }

    // Command queue must be set up first so invalidation
    // commands are available for subsequent steps if needed.
    let mut cq: Queue<16> = Queue::new();
    let cq_base = registers::QueueBase::new((cq.base_paddr() >> 12) as u64, cq.log2sz_minus_1());
    let mut iommu_regs = registers::IOMMU_REGS.get().unwrap().lock();
    iommu_regs.cqb.as_mut_ptr().write(cq_base.value());
    iommu_regs.cqt.as_mut_ptr().write(0);
    iommu_regs.cqcsr.as_mut_ptr().write(registers::CQCSR_CQEN);

    // CQON should be asserted synchronously after CQEN is written;
    // give the hardware one pause, then bail if it isn't ready.
    core::hint::spin_loop();
    if iommu_regs.cqcsr.as_ptr().read() & registers::CQCSR_CQON == 0 {
        warn!("Command queue did not become operational, disabling DMA remapping");
        return;
    }
    drop(iommu_regs);

    super::fault::init();

    let mut ddt = DdtTable::new();
    let page_table = PageTable::<IommuPtConfig>::empty();
    // TODO: Support multiple `device_id` values by iterating over the PCIe
    // bus hierarchy (similar to x86's `PciDeviceLocation::all()`). Currently
    // only `device_id=0` is configured, which covers simple QEMU virt setups
    // but not real hardware with multiple PCIe endpoints.
    if let Err(e) = ddt.enable_device(0, &page_table) {
        warn!("Failed to enable device 0 in DDT: {:?}", e);
        return;
    }

    // TODO: Set `DC.tc.SADE=1` and `DC.tc.GADE=1` to enable hardware-managed
    // Accessed/Dirty bit updates when `capabilities.AMO_HWAD` is set. This
    // avoids the need for software A/D-bit tracking on second-stage PTEs.
    // TODO: Write QoS ID into `DC.ta.RCID`/`MCID` when `capabilities.QOSID`
    // is set, so IOMMU-initiated page table walks are tagged with a QoS
    // identifier for memory system prioritization.

    // Activate the DDT by writing `ddtp`.
    let mut iommu_regs = registers::IOMMU_REGS.get().unwrap().lock();

    let mut ddtp = registers::Ddtp::new();
    ddtp.set_mode(registers::DDTP_MODE_2LVL as u8);
    ddtp.set_ppn((ddt.root_paddr() >> 12) as u64);
    iommu_regs.ddtp.as_mut_ptr().write(ddtp.value());

    // An IOFENCE.C with PR=PW=1 ensures the `ddtp` write is globally
    // visible and ordered before subsequent operations.
    let cmd = queue::cmd_iofence_c(true, true);
    cq.push(&cmd);
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    iommu_regs.cqt.as_mut_ptr().write(cq.tail() as u32);
    drop(iommu_regs);

    // Transfer ownership to global statics so memory stays alive while the
    // IOMMU is active.
    DDT_TABLE.call_once(|| SpinLock::new(ddt));
    PAGE_TABLE.call_once(|| SpinLock::new(page_table));
    COMMAND_QUEUE.call_once(|| SpinLock::new(cq));

    info!("DMA remapping enabled (Sv39x4)");
}

/// Device Directory Table singleton, initialized by [`init`].
static DDT_TABLE: Once<SpinLock<DdtTable, LocalIrqDisabled>> = Once::new();

/// Shared second-stage page table for all devices, initialized by [`init`].
static PAGE_TABLE: Once<SpinLock<PageTable<IommuPtConfig>, LocalIrqDisabled>> = Once::new();

/// Command queue singleton for invalidation and fencing commands.
static COMMAND_QUEUE: Once<SpinLock<Queue<16>, LocalIrqDisabled>> = Once::new();
