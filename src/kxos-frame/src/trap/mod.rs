mod handler;
mod irq;

pub use self::irq::{IrqCallbackHandle, IrqLine};
use core::mem::size_of_val;

use crate::{x86_64_util::*, *};

core::arch::global_asm!(include_str!("trap.S"));
core::arch::global_asm!(include_str!("vector.S"));

#[derive(Debug, Default, Clone, Copy)]
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

#[derive(Debug, Default, Clone, Copy)]
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

pub(crate) fn init() {
    static mut GDT: [usize; 7] = [
        0,
        0x00209800_00000000, // KCODE, EXECUTABLE | USER_SEGMENT | PRESENT | LONG_MODE
        0x00009200_00000000, // KDATA, DATA_WRITABLE | USER_SEGMENT | PRESENT
        0x0000F200_00000000, // UDATA, DATA_WRITABLE | USER_SEGMENT | USER_MODE | PRESENT
        0x0020F800_00000000, // UCODE, EXECUTABLE | USER_SEGMENT | USER_MODE | PRESENT | LONG_MODE
        0,
        0, // TSS, filled in runtime
    ];
    let ptr = unsafe { TSS.as_ptr() as usize };
    let low = (1 << 47)
        | 0b1001 << 40
        | (TSS_SIZE - 1)
        | ((ptr & ((1 << 24) - 1)) << 16)
        | (((ptr >> 24) & ((1 << 8) - 1)) << 56);
    let high = ptr >> 32;
    unsafe {
        GDT[5] = low;
        GDT[6] = high;
        lgdt(&DescriptorTablePointer {
            limit: size_of_val(&GDT) as u16 - 1,
            base: GDT.as_ptr() as _,
        });
    }

    x86_64_util::set_cs((1 << 3) | x86_64_util::RING0);
    x86_64_util::set_ss((2 << 3) | x86_64_util::RING0);

    load_tss((5 << 3) | RING0);
    set_msr(EFER_MSR, get_msr(EFER_MSR) | 1); // enable system call extensions
    set_msr(STAR_MSR, (2 << 3 << 48) | (1 << 3 << 32));
    set_msr(LSTAR_MSR, syscall_entry as _);
    set_msr(SFMASK_MSR, 0x47700); // TF|DF|IF|IOPL|AC|NT

    #[repr(C, align(16))]
    struct IDT {
        entries: [[usize; 2]; 256],
    }
    static mut IDT: IDT = zero();
    let cs = (1 << 3) | x86_64_util::RING0 as usize;
    for i in 0..256 {
        let p = unsafe { __vectors[i] };
        let low = (((p >> 16) & 0xFFFF) << 48)
            | (0b1000_1110_0000_0000 << 32)
            | (cs << 16)
            | (p & 0xFFFF);
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
