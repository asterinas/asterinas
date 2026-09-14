// SPDX-License-Identifier: MPL-2.0

//! NVMe Controller Register definitions.
//!
//! Refer to NVM Express Base Specification Revision 2.0, Section 3.1.

/// 32-bit Controller Registers.
#[repr(usize)]
#[derive(Clone, Copy, Debug)]
#[expect(dead_code)]
pub(crate) enum NvmeRegs32 {
    /// Version (VS): Indicates the NVMe specification version.
    Vs = 0x8,
    /// Interrupt Mask Set (INTMS): Used to set interrupt mask bits.
    Intms = 0xC,
    /// Interrupt Mask Clear (INTMC): Used to clear interrupt mask bits.
    Intmc = 0x10,
    /// Controller Configuration (CC): Used to configure the controller.
    Cc = 0x14,
    /// Controller Status (CSTS): Reports status of the controller.
    Csts = 0x1C,
    /// NVM Subsystem Reset (NSSR): Used to reset the NVM subsystem.
    Nssr = 0x20,
    /// Admin Queue Attributes (AQA): Defines the size of Admin Queues.
    Aqa = 0x24,
    /// Controller Memory Buffer Location (CMBLOC): Indicates the location of the Controller Memory Buffer.
    Cmbloc = 0x38,
    /// Controller Memory Buffer Size (CMBSZ): Indicates the size of the Controller Memory Buffer.
    Cmbsz = 0x3C,
    /// Boot Partition Information (BPINFO): Provides information about boot partitions.
    Bpinfo = 0x40,
    /// Boot Partition Read Select (BPRSEL): Selects which boot partition to read from.
    Bprsel = 0x44,
    /// Boot Partition Memory Buffer Location (BPMBL): Indicates the location of the boot partition memory buffer.
    Bpmbl = 0x48,
    /// Controller Memory Buffer Status (CMBSTS): Reports the status of the Controller Memory Buffer.
    Cmbsts = 0x58,
    /// Persistent Memory Region Capabilities (PMRCAP): Reports Persistent Memory Region capabilities.
    Pmrcap = 0xE00,
    /// Persistent Memory Region Control (PMRCTL): Controls the Persistent Memory Region.
    Pmrctl = 0xE04,
    /// Persistent Memory Region Status (PMRSTS): Reports the status of the Persistent Memory Region.
    Pmrsts = 0xE08,
    /// Persistent Memory Region Elasticity Buffer Size (PMREBS): Reports the size of the elasticity buffer.
    Pmrebs = 0xE0C,
    /// Persistent Memory Region Sustained Write Throughput (PMRSWTP): Reports sustained write throughput.
    Pmrswtp = 0xE10,
}

/// 64-bit Controller Registers.
#[repr(usize)]
#[derive(Clone, Copy, Debug)]
#[expect(dead_code)]
pub(crate) enum NvmeRegs64 {
    /// Controller Capabilities (CAP): Identifies basic capabilities.
    Cap = 0x0,
    /// Admin Submission Queue Base Address (ASQ): Base address of the Admin Submission Queue.
    Asq = 0x28,
    /// Admin Completion Queue Base Address (ACQ): Base address of the Admin Completion Queue.
    Acq = 0x30,
    /// Controller Memory Buffer Memory Space Control (CMBMSC): Controls the Controller Memory Buffer memory space.
    Cmbmsc = 0x50,
    /// Persistent Memory Region Memory Space Control (PMRMSC): Controls the Persistent Memory Region memory space.
    Pmrmsc = 0xE14,
}

/// Exclusive end offset of fixed 32/64-bit registers this driver accesses.
pub(crate) const NVME_BAR0_FIXED_REGS_END: u64 = NvmeRegs64::Pmrmsc as u64 + 8;

impl NvmeRegs64 {
    /// CAP.DSTRD bit shift.
    pub(crate) const CAP_DSTRD_SHIFT: u32 = 32;
    /// CAP.DSTRD bit mask.
    pub(crate) const CAP_DSTRD_MASK: u64 = 0b1111;
    /// CAP.TO bit shift.
    pub(crate) const CAP_TO_SHIFT: u32 = 24;
    /// CAP.TO bit mask.
    pub(crate) const CAP_TO_MASK: u64 = 0xff;
    /// CAP.MPSMIN bit shift.
    pub(crate) const CAP_MPSMIN_SHIFT: u32 = 48;
    /// CAP.MPSMIN bit mask.
    pub(crate) const CAP_MPSMIN_MASK: u64 = 0b1111;
    /// CAP.MPSMAX bit shift.
    pub(crate) const CAP_MPSMAX_SHIFT: u32 = 52;
    /// CAP.MPSMAX bit mask.
    pub(crate) const CAP_MPSMAX_MASK: u64 = 0b1111;
}

/// Doorbell Registers.
///
/// Doorbell registers are used to notify the controller of updates to submission
/// and completion queues. Each queue pair has two doorbell registers:
/// - Submission Queue y Tail Doorbell (SQyTDBL): offset 0x1000 + (2y * (4 << DSTRD))
/// - Completion Queue y Head Doorbell (CQyHDBL): offset 0x1000 + ((2y+1) * (4 << DSTRD))
///
/// Where 'y' is the queue identifier (queue ID).
#[repr(usize)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum NvmeDoorbellRegs {
    /// Submission Queue y Tail Doorbell (SQyTDBL).
    Sqtdbl,
    /// Completion Queue y Head Doorbell (CQyHDBL).
    Cqhdbl,
}

impl NvmeDoorbellRegs {
    /// Calculates the offset for this doorbell register.
    ///
    /// # Arguments
    ///
    /// * `qid` - Queue identifier (queue ID)
    /// * `dstrd` - Doorbell Stride value from the CAP register
    ///
    /// # Returns
    ///
    /// The offset, in bytes, from the base of the controller registers.
    pub(crate) fn offset(&self, qid: u16, dstrd: u16) -> usize {
        const DOORBELL_BASE: usize = 0x1000;
        let stride = 4usize << usize::from(dstrd);

        match self {
            NvmeDoorbellRegs::Sqtdbl => DOORBELL_BASE + (2 * usize::from(qid)) * stride,
            NvmeDoorbellRegs::Cqhdbl => DOORBELL_BASE + ((2 * usize::from(qid)) + 1) * stride,
        }
    }
}
