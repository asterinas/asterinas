// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;
use core::{fmt::Debug, ptr::NonNull};

use bitflags::bitflags;
use log::{error, info};
use spin::Once;
use volatile::{access::ReadWrite, VolatileRef};

use super::registers::Capability;
use crate::{
    arch::trap::TrapFrame,
    sync::{LocalIrqDisabled, SpinLock},
    trap::irq::IrqLine,
};

#[derive(Debug)]
pub struct FaultEventRegisters {
    status: VolatileRef<'static, u32, ReadWrite>,
    /// bit31: Interrupt Mask; bit30: Interrupt Pending.
    _control: VolatileRef<'static, u32, ReadWrite>,
    _data: VolatileRef<'static, u32, ReadWrite>,
    _address: VolatileRef<'static, u32, ReadWrite>,
    _upper_address: VolatileRef<'static, u32, ReadWrite>,
    recordings: Vec<VolatileRef<'static, u128, ReadWrite>>,

    _fault_irq: IrqLine,
}

impl FaultEventRegisters {
    pub fn status(&self) -> FaultStatus {
        FaultStatus::from_bits_truncate(self.status.as_ptr().read())
    }

    /// Creates an instance from the IOMMU base address.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the base address is a valid IOMMU base address and that it has
    /// exclusive ownership of the IOMMU fault event registers.
    unsafe fn new(base_register_vaddr: NonNull<u8>) -> Self {
        // SAFETY: The safety is upheld by the caller.
        let (capability, status, mut control, mut data, mut address, upper_address) = unsafe {
            let base = base_register_vaddr;
            (
                // capability
                VolatileRef::new_read_only(base.add(0x08).cast::<u64>()),
                // status
                VolatileRef::new(base.add(0x34).cast::<u32>()),
                // control
                VolatileRef::new(base.add(0x38).cast::<u32>()),
                // data
                VolatileRef::new(base.add(0x3c).cast::<u32>()),
                // address
                VolatileRef::new(base.add(0x40).cast::<u32>()),
                // upper_address
                VolatileRef::new(base.add(0x44).cast::<u32>()),
            )
        };

        let capability_val = Capability::new(capability.as_ptr().read());
        let length = capability_val.fault_recording_number() as usize + 1;
        let offset = (capability_val.fault_recording_register_offset() as usize) * 16;

        // FIXME: We now trust the hardware. We should instead find a way to check that `length`
        // and `offset` are reasonable values before proceeding.

        let mut recordings = Vec::with_capacity(length);
        for i in 0..length {
            // SAFETY: The safety is upheld by the caller and the correctness of the capability
            // value.
            recordings.push(unsafe {
                VolatileRef::new(base_register_vaddr.add(offset).add(i * 16).cast::<u128>())
            })
        }

        let mut fault_irq = IrqLine::alloc().unwrap();
        fault_irq.on_active(iommu_fault_handler);

        // Set page fault interrupt vector and address
        data.as_mut_ptr().write(fault_irq.num() as u32);
        address.as_mut_ptr().write(0xFEE0_0000);
        control.as_mut_ptr().write(0);

        FaultEventRegisters {
            status,
            _control: control,
            _data: data,
            _address: address,
            _upper_address: upper_address,
            recordings,
            _fault_irq: fault_irq,
        }
    }
}

pub struct FaultRecording(u128);

impl FaultRecording {
    pub fn is_fault(&self) -> bool {
        self.0 & (1 << 127) != 0
    }

    pub fn clear_fault(&mut self) {
        self.0 &= !(1 << 127);
    }

    pub fn request_type(&self) -> FaultRequestType {
        // bit 126 and bit 92
        let t1 = ((self.0 & (1 << 126)) >> 125) as u8;
        let t2 = ((self.0 & (1 << 92)) >> 92) as u8;
        let typ = t1 + t2;
        match typ {
            0 => FaultRequestType::Write,
            1 => FaultRequestType::Page,
            2 => FaultRequestType::Read,
            3 => FaultRequestType::AtomicOp,
            _ => unreachable!(),
        }
    }

    pub fn address_type(&self) -> FaultAddressType {
        match self.0 & (3 << 124) {
            0 => FaultAddressType::UntranslatedRequest,
            1 => FaultAddressType::TranslationRequest,
            2 => FaultAddressType::TranslatedRequest,
            _ => unreachable!(),
        }
    }

    pub fn source_identifier(&self) -> u16 {
        // bit 79:64
        ((self.0 & 0xFFFF_0000_0000_0000_0000) >> 64) as u16
    }

    /// If fault reason is one of the address translation fault conditions, this field contains bits 63:12
    /// of the page address in the faulted request.
    ///
    /// If fault reason is interrupt-remapping fault conditions other than fault reash 0x25, bits 63:48
    /// indicate the interrupt index computed for the faulted interrupt request, and bits 47:12 are cleared.
    ///
    /// If fault reason is interrupt-remapping fault conditions of blocked compatibility mode interrupt (fault reason 0x25),
    /// this field is undefined.
    pub fn fault_info(&self) -> u64 {
        // bit 63:12
        ((self.0 & 0xFFFF_FFFF_FFFF_F000) >> 12) as u64
    }

    #[expect(dead_code)]
    pub fn pasid_value(&self) -> u32 {
        // bit 123:104
        ((self.0 & 0x00FF_FFF0_0000_0000_0000_0000_0000_0000) >> 104) as u32
    }

    pub fn fault_reason(&self) -> u8 {
        // bit 103:96
        ((self.0 & 0xF_0000_0000_0000_0000_0000_0000) >> 96) as u8
    }

    #[expect(dead_code)]
    pub fn pasid_present(&self) -> bool {
        // bit 95
        (self.0 & 0x8000_0000_0000_0000_0000_0000) != 0
    }

    #[expect(dead_code)]
    pub fn execute_permission_request(&self) -> bool {
        // bit 94
        (self.0 & 0x4000_0000_0000_0000_0000_0000) != 0
    }

    #[expect(dead_code)]
    pub fn privilege_mode_request(&self) -> bool {
        // bit 93
        (self.0 & 0x2000_0000_0000_0000_0000_0000) != 0
    }
}

impl Debug for FaultRecording {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FaultRecording")
            .field("Fault", &self.is_fault())
            .field("Request type", &self.request_type())
            .field("Address type", &self.address_type())
            .field("Source identifier", &self.source_identifier())
            .field("Fault Reason", &self.fault_reason())
            .field("Fault info", &self.fault_info())
            .field("Raw", &self.0)
            .finish()
    }
}

#[derive(Debug)]
#[repr(u8)]
pub enum FaultRequestType {
    Write = 0,
    Page = 1,
    Read = 2,
    AtomicOp = 3,
}

#[derive(Debug)]
#[repr(u8)]
#[expect(clippy::enum_variant_names)]
pub enum FaultAddressType {
    UntranslatedRequest = 0,
    TranslationRequest = 1,
    TranslatedRequest = 2,
}

bitflags! {
    pub struct FaultStatus : u32{
        /// Primary Fault Overflow, indicates overflow of the fault recording registers.
        const PFO = 1 << 0;
        /// Primary Pending Fault, indicates there are one or more pending faults logged in the fault recording registers.
        const PPF = 1 << 1;
        /// Invalidation Queue Error.
        const IQE = 1 << 4;
        /// Invalidation Completion Error. Hardware received an unexpected or invalid Device-TLB invalidation completion.
        const ICE = 1 << 5;
        /// Invalidation Time-out Error. Hardware detected a Device-TLB invalidation completion time-out.
        const ITE = 1 << 6;
        /// Fault Record Index, valid only when PPF field is set. This field indicates the index (from base) of the fault recording register
        /// to which the first pending fault was recorded when the PPF field was Set by hardware.
        const FRI = (0xFF) << 8;
    }
}

pub(super) static FAULT_EVENT_REGS: Once<SpinLock<FaultEventRegisters, LocalIrqDisabled>> =
    Once::new();

/// Initializes the fault reporting function.
///
/// # Safety
///
/// The caller must ensure that the base address is a valid IOMMU base address and that it has
/// exclusive ownership of the IOMMU fault event registers.
pub(super) unsafe fn init(base_register_vaddr: NonNull<u8>) {
    FAULT_EVENT_REGS
        // SAFETY: The safety is upheld by the caller.
        .call_once(|| SpinLock::new(unsafe { FaultEventRegisters::new(base_register_vaddr) }));
}

fn iommu_fault_handler(_frame: &TrapFrame) {
    let mut fault_event_regs = FAULT_EVENT_REGS.get().unwrap().lock();

    primary_fault_handler(&mut fault_event_regs);

    let fault_status = fault_event_regs.status();
    if fault_status.intersects(FaultStatus::IQE | FaultStatus::ICE | FaultStatus::ITE) {
        panic!(
            "Catch IOMMU invalidation error. Fault status: {:x?}",
            fault_status
        );
    }
}

fn primary_fault_handler(fault_event_regs: &mut FaultEventRegisters) {
    let mut fault_status = fault_event_regs.status();
    if !fault_status.contains(FaultStatus::PPF) {
        return;
    }

    let start_index = ((fault_event_regs.status().bits & FaultStatus::FRI.bits) >> 8) as usize;
    let mut fault_iter = fault_event_regs.recordings.iter_mut();
    fault_iter.advance_by(start_index).unwrap();
    for raw_recording in fault_iter {
        let raw_recording = raw_recording.as_mut_ptr();
        let mut recording = FaultRecording(raw_recording.read());
        if !recording.is_fault() {
            break;
        }

        // Report
        error!(
            "Catch iommu page fault, doing nothing. recording:{:x?}",
            recording
        );

        // Clear Fault field
        recording.clear_fault();
        raw_recording.write(recording.0);
    }

    if fault_status.contains(FaultStatus::PFO) {
        info!("Primary fault overflow detected.");
        fault_status.remove(FaultStatus::PFO);
        fault_event_regs
            .status
            .as_mut_ptr()
            .write(fault_status.bits);
    }
}
