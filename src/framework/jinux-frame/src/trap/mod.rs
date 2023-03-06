mod handler;
mod irq;

use crate::cell::Cell;
use lazy_static::lazy_static;
use x86_64::{
    registers::{
        model_specific::{self, EferFlags},
        rflags::RFlags,
    },
    structures::{gdt::*, tss::TaskStateSegment},
};

pub use self::irq::{allocate_irq, IrqAllocateHandle};
pub(crate) use self::irq::{allocate_target_irq, IrqCallbackHandle, IrqLine};
use core::{fmt::Debug, mem::size_of_val};

use crate::{x86_64_util::*, *};

core::arch::global_asm!(include_str!("trap.S"));
core::arch::global_asm!(include_str!("vector.S"));

#[derive(Default, Clone, Copy)]
#[repr(C)]
pub struct CallerRegs {
    pub rax: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
}

impl Debug for CallerRegs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!("rax: 0x{:x}, rcx: 0x{:x}, rdx: 0x{:x}, rsi: 0x{:x}, rdi: 0x{:x}, r8: 0x{:x}, r9: 0x{:x}, r10: 0x{:x}, r11: 0x{:x}", 
        self.rax, self.rcx, self.rdx, self.rsi, self.rdi, self.r8, self.r9, self.r10, self.r11))?;
        Ok(())
    }
}

#[derive(Default, Clone, Copy)]
#[repr(C)]
pub struct CalleeRegs {
    pub rsp: u64,
    pub rbx: u64,
    pub rbp: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
}

impl Debug for CalleeRegs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!("rsp: 0x{:x}, rbx: 0x{:x}, rbp: 0x{:x}, r12: 0x{:x}, r13: 0x{:x}, r14: 0x{:x}, r15: 0x{:x}", self.rsp, self.rbx, self.rbp, self.r12, self.r13, self.r14, self.r15))?;
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub struct SyscallFrame {
    pub caller: CallerRegs,
    pub callee: CalleeRegs,
}

#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub struct TrapFrame {
    pub cr2: u64,
    pub caller: CallerRegs,
    // do not use the rsp inside the callee, use another rsp instead
    pub callee: CalleeRegs,
    pub id: u64,
    pub err: u64,
    // Pushed by CPU
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

const TSS_SIZE: usize = 104;

extern "C" {
    /// TSS
    static TSS: [u8; TSS_SIZE];
    /// 所有的中断向量push一个id后跳转到trao_entry
    static __vectors: [usize; 256];
    fn syscall_entry();
}

lazy_static! {
    static ref GDT: Cell<GlobalDescriptorTable> = Cell::new(GlobalDescriptorTable::new());
}

#[repr(C, align(16))]
struct IDT {
    /**
     * The structure of all entries in IDT are shown below:
     * related link: https://wiki.osdev.org/IDT#Structure_on_x86-64
     * Low 64 bits of entry:
     * |0-------------------------------------15|16------------------------------31|
     * |     Low 16 bits of target address      |         Segment Selector         |
     * |32-34|35------39|40-------43|44|45-46|47|48------------------------------63|
     * | IST | Reserved | Gate Type | 0| DPL |P | Middle 16 bits of target address |
     * |---------------------------------------------------------------------------|
     * High 64 bits of entry:
     * |64-----------------------------------------------------------------------95|
     * |                       High 32 bits of target address                      |
     * |96----------------------------------------------------------------------127|
     * |                                 Reserved                                  |
     * |---------------------------------------------------------------------------|
     */
    entries: [[usize; 2]; 256],
}

impl IDT {
    const fn default() -> Self {
        Self {
            entries: [[0; 2]; 256],
        }
    }
}

static mut IDT: IDT = IDT::default();

pub(crate) fn init() {
    // FIXME: use GDT in x86_64 crate in

    let tss = unsafe { &*(TSS.as_ptr() as *const TaskStateSegment) };

    let gdt = GDT.get();
    let kcs = gdt.add_entry(Descriptor::kernel_code_segment());
    let kss = gdt.add_entry(Descriptor::kernel_data_segment());
    let uss = gdt.add_entry(Descriptor::user_data_segment());
    let ucs = gdt.add_entry(Descriptor::user_code_segment());
    let tss_load = gdt.add_entry(Descriptor::tss_segment(tss));

    gdt.load();

    x86_64_util::set_cs(kcs.0);
    x86_64_util::set_ss(kss.0);

    load_tss(tss_load.0);

    unsafe {
        // enable syscall extensions
        model_specific::Efer::update(|efer_flags| {
            efer_flags.insert(EferFlags::SYSTEM_CALL_EXTENSIONS);
        });
    }

    model_specific::Star::write(ucs, uss, kcs, kss)
        .expect("error when configure star msr register");
    // set the syscall entry
    model_specific::LStar::write(x86_64::VirtAddr::new(syscall_entry as u64));
    model_specific::SFMask::write(
        RFlags::TRAP_FLAG
            | RFlags::DIRECTION_FLAG
            // | RFlags::INTERRUPT_FLAG
            | RFlags::IOPL_LOW
            | RFlags::IOPL_HIGH
            | RFlags::NESTED_TASK
            | RFlags::ALIGNMENT_CHECK,
    );

    // initialize the trap entry for all irq number
    for i in 0..256 {
        let p = unsafe { __vectors[i] };
        // set gate type to 1110: 64 bit Interrupt Gate, Present bit to 1, DPL to Ring 0
        let p_low = (((p >> 16) & 0xFFFF) << 48) | (p & 0xFFFF);
        let trap_entry_option: usize = 0b1000_1110_0000_0000;
        let low = (trap_entry_option << 32) | ((kcs.0 as usize) << 16) | p_low;
        let high = p >> 32;
        unsafe {
            IDT.entries[i] = [low, high];
        }
    }
    unsafe {
        lidt(&DescriptorTablePointer {
            limit: size_of_val(&IDT) as u16 - 1,
            base: &IDT as *const _ as _,
        })
    }
}

macro_rules! define_cpu_exception {
    ( $( $name: ident = $exception_num: expr ),* ) => {
        $(
            pub const $name : u64 = $exception_num;
        )*
    }
}

define_cpu_exception!(
    DIVIDE_BY_ZERO = 0,
    DEBUG = 1,
    NON_MASKABLE_INTERRUPT = 2,
    BREAKPOINT = 3,
    OVERFLOW = 4,
    BOUND_RANGE_EXCEEDED = 5,
    INVALID_OPCODE = 6,
    DEVICE_NOT_AVAILABLE = 7,
    DOUBLE_FAULT = 8,
    COPROCESSOR_SEGMENT_OVERRUN = 9,
    INVAILD_TSS = 10,
    SEGMENT_NOT_PRESENT = 11,
    STACK_SEGMENT_FAULT = 12,
    GENERAL_PROTECTION_FAULT = 13,
    PAGE_FAULT = 14,
    // 15 reserved
    X87_FLOATING_POINT_EXCEPTION = 16,
    ALIGNMENT_CHECK = 17,
    MACHINE_CHECK = 18,
    SIMD_FLOATING_POINT_EXCEPTION = 19,
    VIRTUALIZATION_EXCEPTION = 20,
    CONTROL_PROTECTION_EXCEPTION = 21,
    // 22-27 reserved
    HYPERVISOR_INJECTION_EXCEPTION = 28,
    VMM_COMMUNICATION_EXCEPTION = 29,
    SECURITY_EXCEPTION = 30 // 31 reserved
);
