//! Emulated LAPIC and IOAPIC device for guest VMs.
//!
//! Use pure software emulate for now.
use super::ioctl::LapicState;
use crate::prelude::*;

pub const IOAPIC_NUM_PINS: usize = 24;

const APIC_MODE_EXTINT: u32 = 0x7;
const APIC_LVT_VECTOR_MASK: u32 = 0xFF;
const APIC_LVT_DELIVERY_MODE_MASK: u32 = 0x700;
const APIC_LVT_SEND_PENDING: u32 = 1 << 12;
const APIC_LVT_INPUT_POLARITY: u32 = 1 << 13;
const APIC_LVT_REMOTE_IRR: u32 = 1 << 14;
const APIC_LVT_LEVEL_TRIGGER: u32 = 1 << 15;
const APIC_LVT_MASKED: u32 = 1 << 16;
const APIC_LINT_MASK: u32 = APIC_LVT_VECTOR_MASK
    | APIC_LVT_DELIVERY_MODE_MASK
    | APIC_LVT_SEND_PENDING
    | APIC_LVT_INPUT_POLARITY
    | APIC_LVT_REMOTE_IRR
    | APIC_LVT_LEVEL_TRIGGER
    | APIC_LVT_MASKED;
const APIC_TIMER_MODE_ONESHOT: u32 = 0b00;
const APIC_TIMER_MODE_PERIODIC: u32 = 0b01;

/// Local APIC state.
#[derive(Debug)]
pub struct Lapic {
    pub id: u32,
    pub ldr: u32, // Logical Destination Register

    /// Task Priority Register: 7:4 = priority threshold
    pub tpr: u8,
    pub ppr: u8, // Processor Priority (derived)

    /// Interrupt Request Register: containes pending interrupts that have not yet been dispatched to the processor
    pub irr: [u32; 8],
    /// In-Service Register: contains interrupts that have been dispatched to the processor but not yet EOIed
    pub isr: [u32; 8],
    pub icr: [u32; 2], // Interrupt Command Register, 64 bits
    pub tmr: [u32; 8], // Trigger Mode Register

    pub lvt_lint0: u32,
    pub lvt_lint1: u32,

    pub timer: ApicTimer,
}

impl Default for Lapic {
    fn default() -> Self {
        Self {
            id: 0,
            ldr: 0,
            tpr: 0,
            ppr: 0,
            irr: [0; 8],
            isr: [0; 8],
            icr: [0; 2],
            tmr: [0; 8],
            lvt_lint0: APIC_LVT_MASKED,
            lvt_lint1: APIC_LVT_MASKED,
            timer: ApicTimer::default(),
        }
    }
}

/// Intel® 64 and IA-32 Architectures Software Developer’s Manual.
/// 12.5.4. APIC Timer.
/// APIC timer state.
#[derive(Debug, Default)]
pub struct ApicTimer {
    pub lvt_timer: u32, // LVT(Local Vector Table) Timer Register
    /// divide configuration register
    /// timer freq = tsc freq / divide
    pub divide: u32,
    pub initial_count: u32,
    pub current_count: u32,
    pub deadline_tsc: Option<u64>,
}

impl ApicTimer {
    pub fn divide_shift(&self) -> u32 {
        let shift = (self.divide & 0b11) | ((self.divide & 0b1000) >> 1);
        (shift + 1) & 0b111
    }

    pub fn count_to_tsc_cycles(&self, count: u64) -> u64 {
        (count << self.divide_shift()) * (tsc_freq().saturating_add(500_000)) / 1_000_000
    }

    fn is_masked(&self) -> bool {
        (self.lvt_timer & APIC_LVT_MASKED) != 0
    }

    fn mode(&self) -> u32 {
        (self.lvt_timer >> 17) & 0b11
    }

    fn vector(&self) -> u8 {
        (self.lvt_timer & 0xff) as u8
    }

    fn period_tsc_cycles(&self) -> Option<u64> {
        if self.initial_count == 0 {
            return None;
        }
        Some(
            self.count_to_tsc_cycles(u64::from(self.initial_count))
                .max(1),
        )
    }

    fn arm(&mut self, current_tsc: u64, initial_count: u64) {
        self.initial_count = initial_count as u32;
        self.current_count = self.initial_count;
        self.deadline_tsc = self
            .period_tsc_cycles()
            .map(|period| current_tsc.saturating_add(period));
    }

    fn stop(&mut self) {
        self.current_count = 0;
        self.deadline_tsc = None;
    }

    fn current_count(&self, current_tsc: u64) -> u64 {
        let Some(deadline_tsc) = self.deadline_tsc else {
            return 0;
        };
        if current_tsc >= deadline_tsc {
            return 0;
        }
        (deadline_tsc - current_tsc) / self.count_to_tsc_cycles(1).max(1)
    }
}

/// A single I/O APIC redirection table entry.
#[derive(Debug, Default, Clone, Copy)]
pub struct IoapicRedent {
    pub vector: u8,
    /// Delivery mode (3 bits): 000 = Fixed
    pub delivery_mode: u8,
    /// Destination mode: 0 = Physical, 1 = Logical
    pub dest_mode: u8,
    pub remote_irr: bool,
    /// Trigger mode: 0 = Edge, 1 = Level
    pub trigger_mode: bool,
    pub mask: bool,
    /// Target LAPIC ID
    pub dest_id: u8,
}

/// Packed 64-bit redirection table entry (fields + bits view).
#[derive(Debug, Default, Clone, Copy)]
pub struct IoapicRedtbl {
    pub bits: u64,
}

/// I/O APIC state.
#[derive(Debug)]
pub struct Ioapic {
    // reference to 82093AA I/O ADVANCED PROGRAMMABLE INTERRUPT CONTROLLER
    // IOAPIC registers
    // 3.1. IOREGSEL and IOWIN is memory mapped registers
    pub ioregsel: u32,
    // 3.2. IOAPICID and others
    pub id: u32,
    // 3.2.4. IOREDTBL[23:0] -- I/O REDIRECTION TABLE REGISTERS
    pub redtbl: [IoapicRedtbl; IOAPIC_NUM_PINS],
}

impl Default for Ioapic {
    fn default() -> Self {
        Self {
            ioregsel: 0,
            id: 1,
            redtbl: [IoapicRedtbl::default(); IOAPIC_NUM_PINS],
        }
    }
}

impl Lapic {
    pub fn to_kvm_state(&self) -> LapicState {
        let mut state = LapicState::default();

        write_apic_reg(&mut state.regs, XLAPIC_RW_ID, self.id << 24);
        write_apic_reg(&mut state.regs, XLAPIC_RO_VER, (6 << 16) | 0x14);
        write_apic_reg(&mut state.regs, XLAPIC_RW_TPR, self.tpr as u32);
        write_apic_reg(&mut state.regs, XLAPIC_RO_PPR, self.ppr as u32);
        write_apic_reg(&mut state.regs, XLAPIC_RW_LDR, self.ldr);
        write_apic_reg(&mut state.regs, XLAPIC_RW_DFR, 0xFFFF_FFFF);
        write_apic_reg(&mut state.regs, XLAPIC_RW_SIVR, 0x1FF);
        write_apic_reg(&mut state.regs, XLAPIC_RW_LVT_CMCI, APIC_LVT_MASKED);
        write_apic_reg(&mut state.regs, XLAPIC_RW_LVT_THERM, APIC_LVT_MASKED);
        write_apic_reg(&mut state.regs, XLAPIC_RW_LVT_PERF, APIC_LVT_MASKED);
        write_apic_reg(&mut state.regs, XLAPIC_RW_LVT_LINT0, self.lvt_lint0);
        write_apic_reg(&mut state.regs, XLAPIC_RW_LVT_LINT1, self.lvt_lint1);
        write_apic_reg(&mut state.regs, XLAPIC_RW_LVT_ERROR, APIC_LVT_MASKED);
        write_apic_reg(&mut state.regs, XLAPIC_RW_LVT_TIMER, self.timer.lvt_timer);
        write_apic_reg(
            &mut state.regs,
            XLAPIC_RW_TIMER_INIT,
            self.timer.initial_count,
        );
        write_apic_reg(
            &mut state.regs,
            XLAPIC_RO_TIMER_CURR,
            self.timer.current_count,
        );
        write_apic_reg(&mut state.regs, XLAPIC_RW_TIMER_DIVI, self.timer.divide);
        write_apic_reg(&mut state.regs, XLAPIC_RW_ICR_LOW, self.icr[0]);
        write_apic_reg(&mut state.regs, XLAPIC_RW_ICR_HIGH, self.icr[1]);

        for index in 0..8 {
            write_apic_reg(
                &mut state.regs,
                apic_reg_array_offset(XLAPIC_RO_ISR_BASE, index),
                self.isr[index],
            );
            write_apic_reg(
                &mut state.regs,
                apic_reg_array_offset(XLAPIC_RO_TMR_BASE, index),
                self.tmr[index],
            );
            write_apic_reg(
                &mut state.regs,
                apic_reg_array_offset(XLAPIC_RO_IRR_BASE, index),
                self.irr[index],
            );
        }

        state
    }

    pub fn set_from_kvm_state(&mut self, state: &LapicState) {
        self.id = (read_apic_reg(&state.regs, XLAPIC_RW_ID) >> 24) & 0xFF;
        self.tpr = (read_apic_reg(&state.regs, XLAPIC_RW_TPR) & 0xFF) as u8;
        self.ldr = read_apic_reg(&state.regs, XLAPIC_RW_LDR) & 0xFF00_0000;
        self.lvt_lint0 = sanitize_lvt_lint(read_apic_reg(&state.regs, XLAPIC_RW_LVT_LINT0));
        self.lvt_lint1 = sanitize_lvt_lint(read_apic_reg(&state.regs, XLAPIC_RW_LVT_LINT1));
        self.timer.lvt_timer = read_apic_reg(&state.regs, XLAPIC_RW_LVT_TIMER);
        self.timer.initial_count = read_apic_reg(&state.regs, XLAPIC_RW_TIMER_INIT);
        self.timer.current_count = read_apic_reg(&state.regs, XLAPIC_RO_TIMER_CURR);
        self.timer.divide = read_apic_reg(&state.regs, XLAPIC_RW_TIMER_DIVI);
        self.icr[0] = read_apic_reg(&state.regs, XLAPIC_RW_ICR_LOW);
        self.icr[1] = read_apic_reg(&state.regs, XLAPIC_RW_ICR_HIGH);

        for index in 0..8 {
            self.isr[index] = read_apic_reg(
                &state.regs,
                apic_reg_array_offset(XLAPIC_RO_ISR_BASE, index),
            );
            self.tmr[index] = read_apic_reg(
                &state.regs,
                apic_reg_array_offset(XLAPIC_RO_TMR_BASE, index),
            );
            self.irr[index] = read_apic_reg(
                &state.regs,
                apic_reg_array_offset(XLAPIC_RO_IRR_BASE, index),
            );
        }

        self.update_ppr();
    }

    pub fn add_pending_interrupt(&mut self, vec: u8) {
        // Set the corresponding bit in IRR.
        Self::set_bit(&mut self.irr, vec);
    }

    fn complete_interrupt(&mut self) -> Option<u8> {
        if let Some(isr_vec) = Self::find_highest(&self.isr) {
            Self::clear_bit(&mut self.isr, isr_vec);
            self.update_ppr();
            Some(isr_vec)
        } else {
            None
        }
    }

    fn update_ppr(&mut self) {
        let isr_prio = Self::find_highest(&self.isr).map(|v| v & 0xF0).unwrap_or(0) as u8;
        self.ppr = self.tpr.max(isr_prio);
    }

    fn set_bit(val: &mut [u32; 8], vec: u8) {
        val[(vec / 32) as usize] |= 1u32 << (vec % 32);
    }

    fn clear_bit(val: &mut [u32; 8], vec: u8) {
        val[(vec / 32) as usize] &= !(1u32 << (vec % 32));
    }

    fn find_highest(val: &[u32; 8]) -> Option<u8> {
        for i in (0..8usize).rev() {
            let v = val[i];
            if v != 0 {
                let bit = 31 - v.leading_zeros();
                return Some((i as u32 * 32 + bit) as u8);
            }
        }
        None
    }
}

fn sanitize_lvt_lint(value: u32) -> u32 {
    value & APIC_LINT_MASK
}

fn apic_reg_array_offset(base: u64, index: usize) -> u64 {
    base + (index as u64) * 0x10
}

fn read_apic_reg(regs: &[u8], offset: u64) -> u32 {
    let offset = offset as usize;
    let mut bytes = [0; 4];
    bytes.copy_from_slice(&regs[offset..offset + 4]);
    u32::from_le_bytes(bytes)
}

fn write_apic_reg(regs: &mut [u8], offset: u64, value: u32) {
    let offset = offset as usize;
    regs[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

use ostd::{
    arch::{tsc_freq, vm::GuestContext},
    mm::Gpaddr,
    vm::{GuestInterruptPort, GuestPhysMemSpace, GuestTimerPort},
};

impl GuestInterruptPort for Lapic {
    fn check_pending_interrupt(&self) -> Option<u8> {
        let pending_vector = Self::find_highest(&self.irr)?;

        // interrupt vector 的高四位表示优先级，低四位表示具体的中断号。同一优先级的中断由低四位决定先后顺序。
        let pending_prio = pending_vector >> 4;
        let tpr_prio = self.tpr >> 4;
        let isr_prio = Self::find_highest(&self.isr)
            .map(|vector| vector >> 4)
            .unwrap_or(0);

        if pending_prio > tpr_prio && pending_prio > isr_prio {
            Some(pending_vector)
        } else {
            None
        }
    }

    fn accept_interrupt(&mut self, vector: u8) {
        Self::set_bit(&mut self.isr, vector);
        Self::clear_bit(&mut self.irr, vector);
        self.update_ppr();
    }
}

impl GuestTimerPort for Lapic {
    fn check_deadline(&mut self, current_tsc: u64) -> Option<u64> {
        // TODO: The handling of deadline mode, which should be similar to oneshot mode.
        let deadline_tsc = self.timer.deadline_tsc?;
        if current_tsc < deadline_tsc {
            return (!self.timer.is_masked()).then_some(deadline_tsc);
        }

        let vector = self.timer.vector();
        let mode = self.timer.mode();
        let masked = self.timer.is_masked();
        if !masked {
            self.add_pending_interrupt(vector);
        }

        let next_deadline = match mode {
            APIC_TIMER_MODE_PERIODIC => {
                let Some(period) = self.timer.period_tsc_cycles() else {
                    self.timer.stop();
                    return None;
                };
                let elapsed_periods = current_tsc
                    .saturating_sub(deadline_tsc)
                    .checked_div(period)
                    .unwrap_or(0)
                    .saturating_add(1);
                Some(deadline_tsc.saturating_add(period.saturating_mul(elapsed_periods)))
            }
            APIC_TIMER_MODE_ONESHOT => None,
            _ => None,
        };
        self.timer.deadline_tsc = next_deadline;
        next_deadline
    }
}

/// Move an IRR vector into service (set ISR, clear IRR, update PPR).
// fn lapic_kick_to_service(lapic: &mut Lapic, vec: u8) {
//     lapic_set_isr(lapic, vec);
//     lapic_clear_irr(lapic, vec);
//     lapic_update_ppr(lapic);
// }

/// Return the highest-priority pending vector that is deliverable, or `None`.
/// pending vector is the highest set bit in IRR.
/// pending vector is deliverable if its priority is higher than the current TPR and all ISR vectors.
// fn lapic_check_pending_vector(lapic: &Lapic) -> Option<u8> {
//     let pending_vector = lapic_find_highest_irr(lapic)?;

//     let isr_vector = lapic_find_highest_isr(lapic);

//     // interrupt vector 的高四位表示优先级，低四位表示具体的中断号。同一优先级的中断由低四位决定先后顺序。
//     let pending_prio = pending_vector >> 4;
//     let tpr_prio = lapic.tpr >> 4;
//     let isr_prio = isr_vector.map(|v| v >> 4).unwrap_or(0);

//     if pending_prio > tpr_prio && pending_prio > isr_prio {
//         Some(pending_vector)
//     } else {
//         None
//     }
// }

impl Ioapic {
    /// Delivers an IRQ from the I/O APIC to the appropriate vCPU LAPICs.
    pub fn inject_irq_line<'a, I>(&mut self, lapics: I, irq: usize)
    where
        I: IntoIterator<Item = &'a mut Lapic>,
    {
        if irq >= IOAPIC_NUM_PINS {
            return;
        }

        let entry = self.redtbl[irq].fields();

        if entry.mask {
            return;
        }
        if entry.remote_irr {
            return;
        }

        let vec = entry.vector;
        if vec < 0x10 || vec > 0xFE {
            return;
        }

        if entry.delivery_mode != 0b000 {
            warn!(
                "vIOAPIC: Unhandled delivery mode: {:03b}, ignore",
                entry.delivery_mode
            );
        }

        let destination = entry.dest_id;
        let mut delivered = false;
        if entry.dest_mode == 0 {
            // Physical mode: send to a specific LAPIC.
            for lapic in lapics {
                if lapic.id as u8 == destination {
                    lapic.add_pending_interrupt(vec);
                    delivered = true;
                    break;
                }
            }
        } else {
            // Logical mode (flat model): send to a group of LAPICs.
            for lapic in lapics {
                if ((lapic.ldr >> 24) as u8) & destination != 0 {
                    lapic.add_pending_interrupt(vec);
                    delivered = true;
                }
            }
        }

        if delivered && entry.trigger_mode {
            self.redtbl[irq].set_remote_irr(true);
        }
    }

    /// Clear the `remote_irr` flag for any redirection entry whose vector matches `vec`.
    pub fn complete_interrupt(&mut self, vec: u8) {
        for irq in 0..IOAPIC_NUM_PINS {
            if self.redtbl[irq].fields().vector == vec {
                self.redtbl[irq].set_remote_irr(false);
            }
        }
    }
}

impl IoapicRedtbl {
    pub fn fields(&self) -> IoapicRedent {
        IoapicRedent {
            vector: (self.bits & 0xFF) as u8,
            delivery_mode: ((self.bits >> 8) & 0x7) as u8,
            dest_mode: ((self.bits >> 11) & 0x1) as u8,
            remote_irr: ((self.bits >> 14) & 0x1) != 0,
            trigger_mode: ((self.bits >> 15) & 0x1) != 0,
            mask: ((self.bits >> 16) & 0x1) != 0,
            dest_id: ((self.bits >> 56) & 0xFF) as u8,
        }
    }

    pub fn set_remote_irr(&mut self, val: bool) {
        if val {
            self.bits |= 1 << 14;
        } else {
            self.bits &= !(1 << 14);
        }
    }
}

pub const LAPIC_BASE: u64 = 0xFEE0_0000;
pub const LAPIC_SIZE: u64 = 0x400;

pub const IOAPIC_BASE: u64 = 0xFEC0_0000;
pub const IOAPIC_SIZE: u64 = 0x20;

const XLAPIC_RW_ID: u64 = 0x020;
const XLAPIC_RO_VER: u64 = 0x030;
const XLAPIC_RW_TPR: u64 = 0x080;
const XLAPIC_RO_APR: u64 = 0x090;
const XLAPIC_RO_PPR: u64 = 0x0A0;
const XLAPIC_WO_EOI: u64 = 0x0B0;
const XLAPIC_RO_RRD: u64 = 0x0C0;
const XLAPIC_RW_LDR: u64 = 0x0D0;
const XLAPIC_RW_DFR: u64 = 0x0E0;
const XLAPIC_RW_SIVR: u64 = 0x0F0;
const XLAPIC_RO_ISR_BASE: u64 = 0x100;
const XLAPIC_RO_ISR_SIZE: u64 = 0x080; // 0x180 - 0x100
const XLAPIC_RO_TMR_BASE: u64 = 0x180;
const XLAPIC_RO_TMR_SIZE: u64 = 0x080; // 0x200 - 0x180
const XLAPIC_RO_IRR_BASE: u64 = 0x200;
const XLAPIC_RO_IRR_SIZE: u64 = 0x080; // 0x280 - 0x200
const XLAPIC_RW_ESR: u64 = 0x280;
const XLAPIC_RW_LVT_CMCI: u64 = 0x2F0;
const XLAPIC_RW_ICR_LOW: u64 = 0x300;
const XLAPIC_RW_ICR_HIGH: u64 = 0x310;
const XLAPIC_RW_LVT_TIMER: u64 = 0x320;
const XLAPIC_RW_LVT_THERM: u64 = 0x330;
const XLAPIC_RW_LVT_PERF: u64 = 0x340;
const XLAPIC_RW_LVT_LINT0: u64 = 0x350;
const XLAPIC_RW_LVT_LINT1: u64 = 0x360;
const XLAPIC_RW_LVT_ERROR: u64 = 0x370;
const XLAPIC_RW_TIMER_INIT: u64 = 0x380;
const XLAPIC_RO_TIMER_CURR: u64 = 0x390;
const XLAPIC_WO_SELF_IPI: u64 = 0x3F0;
const XLAPIC_RW_TIMER_DIVI: u64 = 0x3E0;

const MAX_INSN_LENGTH: usize = 15;

#[derive(Debug, Clone, Copy)]
struct MmioInstruction {
    is_read: bool,
    size: u8,
    reg: u8,
    imm: Option<u64>,
    len: usize,
}

use super::vcpu::Vcpu;

/// Emulate a guest access to APIC MMIO region.
/// Returns `Ok(true)` if the access is successfully emulated.
///         `Ok(false)` if the access is not to APIC MMIO or is unsupported.
///         `Err` if an error occurs during emulation.
pub(super) fn emulate_apic_mmio(vcpu: Arc<Vcpu>, fault_gpa: u64) -> Result<bool> {
    // log::error!("Guest access to APIC MMIO at GPA {:#x}", fault_gpa);
    let is_lapic = (LAPIC_BASE..(LAPIC_BASE + LAPIC_SIZE)).contains(&fault_gpa);
    let is_ioapic = (IOAPIC_BASE..(IOAPIC_BASE + IOAPIC_SIZE)).contains(&fault_gpa);
    if !is_lapic && !is_ioapic {
        return Ok(false);
    }

    let vm_handle = vcpu.vm()?;
    let guest_mem = vm_handle.guest_mem();
    let mut insn_bytes = [0_u8; MAX_INSN_LENGTH];
    let (guest_rip, guest_rip_gpa) = {
        let context = vcpu.guest_context();
        let guest_rip = context.rip() as usize;
        let guest_rip_gpa = match translate_gva_to_gpa(&context, guest_mem, guest_rip) {
            Ok(gpa) => gpa,
            Err(err) => {
                error!(
                    "hypervisor: failed to translate APIC MMIO instruction RIP {:#x}: {:?}",
                    guest_rip, err
                );
                return Err(err.into());
            }
        };
        (guest_rip, guest_rip_gpa)
    };
    let mut reader = guest_mem.reader(guest_rip_gpa, insn_bytes.len())?;
    if let Err((err, _)) =
        reader.read_fallible(&mut VmWriter::from(insn_bytes.as_mut_slice()).to_fallible())
    {
        error!(
            "hypervisor: failed to read APIC MMIO instruction bytes: rip={:#x}, gpa={:#x}, err={:?}",
            guest_rip, guest_rip_gpa, err
        );
        return Err(err.into());
    }

    let Some(insn) = decode_mmio_instruction(&insn_bytes) else {
        error!(
            "hypervisor: failed to decode APIC MMIO instruction: rip={:#x}, gpa={:#x}, bytes={:02x?}",
            guest_rip, guest_rip_gpa, insn_bytes
        );
        return Ok(false);
    };

    if is_lapic {
        if !emulate_lapic_mmio(vcpu.clone(), fault_gpa, insn)? {
            return Ok(false);
        }
    } else {
        if !emulate_ioapic_mmio(vcpu.clone(), fault_gpa, insn)? {
            return Ok(false);
        }
    }

    vcpu.guest_context().advance_rip(insn.len as u64);
    Ok(true)
}

fn translate_gva_to_gpa(
    context: &GuestContext,
    guest_mem: &GuestPhysMemSpace,
    gva: usize,
) -> core::result::Result<Gpaddr, ostd::Error> {
    const PTE_PRESENT: u64 = 1 << 0;
    const PTE_HUGE: u64 = 1 << 7;
    const PTE_ADDR_MASK: u64 = 0x000f_ffff_ffff_f000;
    const PAGE_2M_MASK: Gpaddr = (1 << 21) - 1;
    const PAGE_1G_MASK: Gpaddr = (1 << 30) - 1;
    const PTE_SIZE: Gpaddr = size_of::<u64>();

    let sregs = context.sregs();
    if (sregs.cr0 & (1 << 31)) == 0 {
        return Ok(gva);
    }

    let read_guest_pte = |gpa: Gpaddr| -> core::result::Result<u64, ostd::Error> {
        let mut reader = guest_mem.reader(gpa, PTE_SIZE)?;
        reader.read_val::<u64>()
    };
    let pte_addr = |entry: u64| -> Gpaddr { (entry & PTE_ADDR_MASK) as Gpaddr };

    let cr3 = (sregs.cr3 as Gpaddr) & !0xfff;
    let pml4e_gpa = cr3 + (((gva >> 39) & 0x1ff) * PTE_SIZE);
    let pml4e = read_guest_pte(pml4e_gpa)?;
    if (pml4e & PTE_PRESENT) == 0 {
        return Err(ostd::Error::PageFault);
    }

    let pdpte = read_guest_pte(pte_addr(pml4e) + (((gva >> 30) & 0x1ff) * PTE_SIZE))?;
    if (pdpte & PTE_PRESENT) == 0 {
        return Err(ostd::Error::PageFault);
    }
    if (pdpte & PTE_HUGE) != 0 {
        return Ok(pte_addr(pdpte) | (gva & PAGE_1G_MASK));
    }

    let pde = read_guest_pte(pte_addr(pdpte) + (((gva >> 21) & 0x1ff) * PTE_SIZE))?;
    if (pde & PTE_PRESENT) == 0 {
        return Err(ostd::Error::PageFault);
    }
    if (pde & PTE_HUGE) != 0 {
        return Ok(pte_addr(pde) | (gva & PAGE_2M_MASK));
    }

    let pte = read_guest_pte(pte_addr(pde) + (((gva >> 12) & 0x1ff) * PTE_SIZE))?;
    if (pte & PTE_PRESENT) == 0 {
        return Err(ostd::Error::PageFault);
    }

    Ok(pte_addr(pte) | (gva & 0xfff))
}

fn emulate_lapic_mmio(vcpu: Arc<Vcpu>, fault_gpa: u64, insn: MmioInstruction) -> Result<bool> {
    // log::error!("Guest access to LAPIC MMIO at GPA {:#x}", fault_gpa);
    let offset = fault_gpa - LAPIC_BASE;
    // read
    if insn.is_read {
        let (value, ok) = emulate_lapic_read(vcpu.clone(), offset);
        if !ok {
            return Ok(false);
        }

        let gpr_index = map_instruction_gpr_index_to_common_gpr_index(insn.reg);
        vcpu.guest_context().set_gpr(gpr_index, insn.size, value);

        return Ok(true);
    }

    // write
    let value = {
        if let Some(value) = insn.imm {
            value
        } else {
            let gpr_index = map_instruction_gpr_index_to_common_gpr_index(insn.reg);
            vcpu.guest_context().gpr(gpr_index)
        }
    };

    let vm = vcpu.vm()?;

    match emulate_lapic_write(vcpu.clone(), offset, value) {
        Some(LapicWriteEffect::Eoi { isr_vec }) => {
            vm.ioapic().complete_interrupt(isr_vec);
        }
        Some(LapicWriteEffect::DeliverIcr { icr }) => {
            vm.inject_ipi(icr)?;
        }
        None => {}
    }
    Ok(true)
}

fn emulate_ioapic_mmio(vcpu: Arc<Vcpu>, fault_gpa: u64, insn: MmioInstruction) -> Result<bool> {
    // log::error!("Guest access to IOAPIC MMIO at GPA {:#x}", fault_gpa);
    let offset = fault_gpa - IOAPIC_BASE;
    // log::error!("IOAPIC MMIO access with offset {:#x}, IOAPIC_BASE {:#x}", offset, IOAPIC_BASE);
    let vm = vcpu.vm()?;
    let mut ioapic = vm.ioapic();

    // read
    if insn.is_read {
        let (value, ok) = emulate_ioapic_read(&ioapic, offset);
        if !ok {
            return Ok(false);
        }
        let gpr_index = map_instruction_gpr_index_to_common_gpr_index(insn.reg);
        vcpu.guest_context().set_gpr(gpr_index, insn.size, value);

        return Ok(true);
    }

    // write
    let value = {
        if let Some(value) = insn.imm {
            value
        } else {
            let gpr_index = map_instruction_gpr_index_to_common_gpr_index(insn.reg);
            vcpu.guest_context().gpr(gpr_index)
        }
    };
    if !emulate_ioapic_write(&mut ioapic, offset, value) {
        return Ok(false);
    }
    Ok(true)
}

/// Emulate a LAPIC MMIO read.
///
/// Returns `(value, ok)` where `ok` is false if the offset is unsupported.
pub fn emulate_lapic_read(vcpu: Arc<Vcpu>, offset: u64) -> (u64, bool) {
    let lapic = vcpu.lapic();
    let value = match offset {
        XLAPIC_RW_ID => (lapic.id as u64) << 24,
        // Not support EOI-broadcast; Max LVT Number is 6; Version is 'Integrated APIC'
        XLAPIC_RO_VER => (0u64 << 24) | (6u64 << 16) | 0x14,
        XLAPIC_RW_TPR => lapic.tpr as u64,
        XLAPIC_RO_APR => 0,
        XLAPIC_RO_PPR => lapic.ppr as u64,
        XLAPIC_RO_RRD => 0,
        XLAPIC_RW_LDR => lapic.ldr as u64,
        XLAPIC_RW_DFR => 0xFFFF_FFFF,
        XLAPIC_RW_SIVR => 0x1FF,
        XLAPIC_RW_LVT_CMCI => 1u64 << 16,
        XLAPIC_RW_LVT_THERM | XLAPIC_RW_LVT_PERF | XLAPIC_RW_LVT_ERROR => 0x10000,
        XLAPIC_RW_LVT_LINT0 => lapic.lvt_lint0 as u64,
        XLAPIC_RW_LVT_LINT1 => lapic.lvt_lint1 as u64,
        XLAPIC_RW_LVT_TIMER => lapic.timer.lvt_timer as u64,
        XLAPIC_RW_TIMER_INIT => lapic.timer.initial_count as u64,
        XLAPIC_RW_TIMER_DIVI => lapic.timer.divide as u64,
        XLAPIC_RO_TIMER_CURR => {
            // read the timer ticks remaining until the next timer interrupt.
            let context = vcpu.guest_context();
            lapic.timer.current_count(context.guest_tsc())
        }
        XLAPIC_RW_ESR => 0,
        o if o >= XLAPIC_RO_ISR_BASE && o < XLAPIC_RO_ISR_BASE + XLAPIC_RO_ISR_SIZE => {
            lapic.isr[((o - XLAPIC_RO_ISR_BASE) / 16) as usize] as u64
        }
        o if o >= XLAPIC_RO_IRR_BASE && o < XLAPIC_RO_IRR_BASE + XLAPIC_RO_IRR_SIZE => {
            lapic.irr[((o - XLAPIC_RO_IRR_BASE) / 16) as usize] as u64
        }
        XLAPIC_RW_ICR_HIGH => lapic.icr[1] as u64,
        XLAPIC_RW_ICR_LOW => lapic.icr[0] as u64,
        o if o >= XLAPIC_RO_TMR_BASE && o < XLAPIC_RO_TMR_BASE + XLAPIC_RO_TMR_SIZE => {
            lapic.tmr[((o - XLAPIC_RO_TMR_BASE) / 16) as usize] as u64
        }
        _ => {
            warn!("MMIO.xLAPIC: Read at offset {:#05x} not supported", offset);
            return (0, false);
        }
    };
    (value, true)
}

pub struct Icr {
    pub delivery_mode: u8,
    pub dest_mode: u8,
    pub dest_shorthand: u8,
    pub dest_id: u8,
    pub src_id: u8,
    pub vector: u8,
}

/// Result of a LAPIC write that may require a timer action.
pub enum LapicWriteEffect {
    Eoi { isr_vec: u8 },
    DeliverIcr { icr: Icr },
}

/// Emulate a LAPIC MMIO write.
///
/// Returns the side-effect the caller must act on, and `ok` (false = unsupported offset).
pub fn emulate_lapic_write(vcpu: Arc<Vcpu>, offset: u64, value: u64) -> Option<LapicWriteEffect> {
    let mut lapic = vcpu.lapic();
    match offset {
        XLAPIC_RW_ID => {
            let new_apic_id = ((value >> 24) & 0xFF) as u32;
            lapic.id = new_apic_id;
        }
        XLAPIC_RW_TPR => {
            lapic.tpr = (value & 0xFF) as u8;
        }
        XLAPIC_WO_EOI => {
            // Find highest in-service vector and complete it
            if let Some(isr_vec) = lapic.complete_interrupt() {
                return Some(LapicWriteEffect::Eoi { isr_vec });
            }
        }
        XLAPIC_RW_LDR => {
            let new = (value as u32) & 0xFF00_0000;
            // Accept only a single-bit logical ID
            if new != 0 && (new & new.wrapping_sub(1 << 24)) == 0 {
                lapic.ldr = new;
            }
        }
        XLAPIC_RW_DFR => {
            if ((value >> 28) & 0xF) != 0xF {
                warn!("vLAPIC: Unsupported cluster model, ignore");
            }
        }
        XLAPIC_RW_LVT_TIMER => {
            lapic.timer.lvt_timer = value as u32;
        }
        XLAPIC_RW_TIMER_INIT => {
            let current_tsc = vcpu.guest_context().guest_tsc();
            lapic.timer.arm(current_tsc, value);
        }
        XLAPIC_RW_TIMER_DIVI => {
            lapic.timer.divide = value as u32;
        }
        XLAPIC_WO_SELF_IPI => {
            lapic.add_pending_interrupt((value & 0xFF) as u8);
        }
        XLAPIC_RW_ESR => {
            if value != 0 {
                warn!(
                    "MMIO.xLAPIC: Write to xLAPIC_RW_ESR with non-zero value {:#018x}",
                    value
                );
            }
        }
        XLAPIC_RW_ICR_LOW => {
            lapic.icr[0] = value as u32;
            // TODO: make sure
            if value >> 32 != 0 {
                lapic.icr[1] = (value >> 32) as u32;
            }
            let icr = Icr {
                vector: (value & 0xFF) as u8,
                delivery_mode: ((value >> 8) & 0x7) as u8,
                dest_mode: ((value >> 11) & 0x1) as u8,
                dest_shorthand: ((value >> 18) & 0b11) as u8,
                dest_id: ((lapic.icr[1] >> 24) & 0xFF) as u8,
                src_id: lapic.id as u8,
            };
            return Some(LapicWriteEffect::DeliverIcr { icr });
        }
        XLAPIC_RW_ICR_HIGH => {
            lapic.icr[1] = value as u32;
        }
        XLAPIC_RW_LVT_LINT0 => {
            lapic.lvt_lint0 = sanitize_lvt_lint(value as u32);
        }
        XLAPIC_RW_LVT_LINT1 => {
            lapic.lvt_lint1 = sanitize_lvt_lint(value as u32);
        }
        XLAPIC_RW_SIVR | XLAPIC_RW_LVT_CMCI | XLAPIC_RW_LVT_THERM | XLAPIC_RW_LVT_PERF
        | XLAPIC_RW_LVT_ERROR => { /* silently ignored */ }
        _ => {
            warn!(
                "MMIO.xLAPIC: Write at offset {:#05x} not supported, value is {:#018x}",
                offset, value
            );
            return None;
        }
    }
    None
}

/// Emulate an IOAPIC MMIO read.
pub fn emulate_ioapic_read(ioapic: &Ioapic, offset: u64) -> (u64, bool) {
    if offset == 0x00 {
        return (ioapic.ioregsel as u64, true);
    } else if offset == 0x10 {
        let index = ioapic.ioregsel;
        let value = match index {
            0x00 => (ioapic.id as u64) << 24,
            0x01 => {
                // Bits 0-7: Version (0x11 for 82093AA)
                // Bits 16-23: Max Redirection Entry (N-1); 24 entries -> 23
                0x0017_0011
            }
            i if (0x10..=0x3F).contains(&i) => {
                let pin = ((i - 0x10) / 2) as usize;
                let value = ioapic.redtbl[pin].bits;
                if i & 1 != 0 {
                    value >> 32
                } else {
                    value & 0x0000_0000_FFFF_FFFF
                }
            }
            _ => 0,
        };
        return (value, true);
    }
    warn!("IOAPIC: Read invalid offset {:#x}", offset);
    (0, false)
}

/// Emulate an IOAPIC MMIO write.
pub fn emulate_ioapic_write(ioapic: &mut Ioapic, offset: u64, value: u64) -> bool {
    if offset == 0x00 {
        ioapic.ioregsel = (value & 0xFF) as u32;
        return true;
    } else if offset == 0x10 {
        let index = ioapic.ioregsel;
        if (0x10..=0x3F).contains(&index) {
            let pin = ((index - 0x10) / 2) as usize;
            if index & 1 != 0 {
                ioapic.redtbl[pin].bits &= 0x0000_0000_FFFF_FFFF;
                ioapic.redtbl[pin].bits |= value << 32;
            } else {
                ioapic.redtbl[pin].bits &= 0xFFFF_FFFF_0000_0000;
                ioapic.redtbl[pin].bits |= value & 0xFFFF_FFFF;
            }
        }
        return true;
    }
    warn!("IOAPIC: Write invalid offset {:#x}", offset);
    false
}

pub fn default_lapic_ldr(vcpu_id: u32) -> u32 {
    (1_u32.checked_shl(vcpu_id).unwrap_or(0)) << 24
}

pub fn default_lapic_lvt_lint0(vcpu_id: u32) -> u32 {
    if vcpu_id == 0 {
        APIC_MODE_EXTINT << 8
    } else {
        APIC_LVT_MASKED
    }
}

const APIC_ICR_DESTINATION_MODE_LOGICAL: u8 = 1;
const APIC_ICR_SHORTHAND_NONE: u8 = 0;
const APIC_ICR_SHORTHAND_SELF: u8 = 1;
const APIC_ICR_SHORTHAND_ALL_INCLUDING_SELF: u8 = 2;
const APIC_ICR_SHORTHAND_ALL_EXCLUDING_SELF: u8 = 3;

pub fn icr_matches_destination(target_lapic: &Lapic, source_icr: &Icr) -> bool {
    match source_icr.dest_shorthand {
        APIC_ICR_SHORTHAND_NONE => {
            if source_icr.dest_mode == APIC_ICR_DESTINATION_MODE_LOGICAL {
                ((target_lapic.ldr >> 24) as u8) & source_icr.dest_id != 0
            } else {
                target_lapic.id as u8 == source_icr.dest_id
            }
        }
        APIC_ICR_SHORTHAND_SELF => target_lapic.id as u8 == source_icr.src_id,
        APIC_ICR_SHORTHAND_ALL_INCLUDING_SELF => true,
        APIC_ICR_SHORTHAND_ALL_EXCLUDING_SELF => target_lapic.id as u8 != source_icr.src_id,
        _ => false,
    }
}

fn decode_mmio_instruction(bytes: &[u8; MAX_INSN_LENGTH]) -> Option<MmioInstruction> {
    let mut ptr = 0usize;
    let mut op_size_16 = false;
    let mut rex = 0u8;
    let mut rex_w = false;

    while ptr < bytes.len() {
        let byte = bytes[ptr];
        match byte {
            0x66 => op_size_16 = true,
            0x67 | 0x2e | 0x36 | 0x3e | 0x26 | 0x64 | 0x65 | 0xf0 | 0xf2 | 0xf3 => {}
            b if (b & 0xf0) == 0x40 => {
                rex = b;
                rex_w = (b & 0x08) != 0;
            }
            _ => break,
        }
        ptr += 1;
    }

    let opcode = *bytes.get(ptr)?;
    ptr += 1;

    // MOV Opcode
    // Volume 2, 4.3 Instructions(M-U)
    let (is_read, size, imm_size) = match opcode {
        0x88 => (false, 1, 0),
        0x8a => (true, 1, 0),
        0x89 => (
            false,
            if rex_w {
                8
            } else if op_size_16 {
                2
            } else {
                4
            },
            0,
        ),
        0x8b => (
            true,
            if rex_w {
                8
            } else if op_size_16 {
                2
            } else {
                4
            },
            0,
        ),
        0xc6 => (false, 1, 1),
        0xc7 => (
            false,
            if op_size_16 { 2 } else { 4 },
            if op_size_16 { 2 } else { 4 },
        ),
        _ => return None,
    };

    // modrm
    // bit: 7 6 | 5 4 3 | 2 1 0
    //      mod |  reg  |  r/m
    // mod: r/m is reg or memory
    //      11    for reg
    //      other for memory
    // Volume 2, 2.1 Instruction Format...
    let modrm = *bytes.get(ptr)?;
    ptr += 1;

    let mode = modrm >> 6;
    let rm = modrm & 0x7;
    if mode == 0b11 {
        return None;
    }
    if rm == 0x4 {
        let sib = *bytes.get(ptr)?;
        ptr += 1;
        let base = sib & 0x7;
        if mode == 0 && base == 0x5 {
            ptr += 4;
        }
    } else if mode == 0 && rm == 0x5 {
        ptr += 4;
    }
    match mode {
        0 => {}
        1 => ptr += 1,
        2 => ptr += 4,
        _ => return None,
    }
    if ptr > MAX_INSN_LENGTH {
        return None;
    }

    let mut reg = (modrm >> 3) & 0x7;
    if (rex & 0x04) != 0 {
        reg |= 0x8;
    }
    let imm = if imm_size != 0 {
        if reg != 0 {
            return None;
        }
        let value = read_le_immediate(bytes, ptr, imm_size)?;
        ptr += imm_size;
        Some(value)
    } else {
        None
    };

    Some(MmioInstruction {
        is_read,
        size,
        reg,
        imm,
        len: ptr,
    })
}

fn read_le_immediate(bytes: &[u8; MAX_INSN_LENGTH], offset: usize, size: usize) -> Option<u64> {
    let end = offset.checked_add(size)?;
    if end > MAX_INSN_LENGTH {
        return None;
    }

    let mut value = 0_u64;
    for (index, byte) in bytes[offset..end].iter().enumerate() {
        value |= u64::from(*byte) << (index * 8);
    }
    Some(value)
}

fn map_instruction_gpr_index_to_common_gpr_index(index: u8) -> u8 {
    // The encode method of gpr index in ModRM:
    // ???
    match index {
        0 => 0,
        1 => 2,
        2 => 3,
        3 => 1,
        4 => 7,
        5 => 6,
        6 => 4,
        7 => 5,
        other => other,
    }
}
