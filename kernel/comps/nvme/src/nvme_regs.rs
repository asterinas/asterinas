// SPDX-License-Identifier: MPL-2.0

//! NVMe Controller Register definitions.
//!
//! Refer to NVM Express Base Specification Revision 2.0, Section 3.1.

/// 32-bit Controller Registers.
#[derive(Copy, Clone, Debug)]
#[repr(C)]
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
#[derive(Copy, Clone, Debug)]
#[repr(C)]
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

/// Doorbell Registers.
///
/// Doorbell registers are used to notify the controller of updates to submission
/// and completion queues. Each queue pair has two doorbell registers:
/// - Submission Queue y Tail Doorbell (SQyTDBL): offset 0x1000 + (2y * (4 << DSTRD))
/// - Completion Queue y Head Doorbell (CQyHDBL): offset 0x1000 + ((2y+1) * (4 << DSTRD))
///
/// Where 'y' is the queue identifier (queue ID).
#[derive(Copy, Clone, Debug)]
pub(crate) enum NvmeDoorBellRegs {
    /// Submission Queue y Tail Doorbell (SQyTDBL).
    Sqtdb,
    /// Completion Queue y Head Doorbell (CQyHDBL).
    Cqhdb,
}

impl NvmeDoorBellRegs {
    /// Calculates the offset for this doorbell register.
    ///
    /// # Arguments
    /// * `qid` - Queue identifier (queue ID)
    /// * `dstrd` - Doorbell Stride value from the CAP register
    ///
    /// # Returns
    /// The calculated offset in bytes from the base of the controller registers.
    pub(crate) fn offset(&self, qid: u16, dstrd: u16) -> u32 {
        const DOORBELL_BASE: u32 = 0x1000;
        let stride = (4 << dstrd) as u32;

        match self {
            NvmeDoorBellRegs::Sqtdb => DOORBELL_BASE + (2 * qid as u32) * stride,
            NvmeDoorBellRegs::Cqhdb => DOORBELL_BASE + ((2 * qid as u32) + 1) * stride,
        }
    }
}
