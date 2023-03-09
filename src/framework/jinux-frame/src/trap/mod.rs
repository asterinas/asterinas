mod handler;
mod irq;

pub(crate) use self::handler::call_irq_callback_functions;
pub use self::irq::{allocate_irq, IrqAllocateHandle};
pub(crate) use self::irq::{allocate_target_irq, IrqCallbackHandle, IrqLine};

pub(crate) fn init() {
    unsafe {
        trapframe::init();
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
