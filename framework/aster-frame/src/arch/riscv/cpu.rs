// SPDX-License-Identifier: MPL-2.0

/// Returns the number of CPUs.
pub fn num_cpus() -> u32 {}

/// Returns the ID of this CPU.
pub fn this_cpu() -> u32 {
    // FIXME: we only start one cpu now.
    0
}

/// The definition conforms to the RISC-V Supervisor-Level ISA Specification:
/// <https://riscv.org/wp-content/uploads/2017/05/riscv-privileged-v1.10.pdf>
/// 
/// The CPU exception defined here makes an abstract over the possible values
/// of the `scause` register.
#[derive(Debug, Eq, PartialEq)]
pub struct CpuException {
    pub number: u16,
    pub typ: CpuExceptionType,
}

#[derive(PartialEq, Eq, Debug)]
pub enum CpuExceptionType {
    Fault,
    Interrupt,
    Reserved,
}

macro_rules! define_cpu_exception_list {
    ( $list_name: ident, $([ $name: ident = $exception_num:tt, $exception_type:tt],)* ) => {
        const $list_name : &[CpuException] = &[
            $($name,)*
        ];
        $(
            pub const $name : CpuException = CpuException{
                number: $exception_num,
                typ: CpuExceptionType::$exception_type,
            };
        )*
    }
}

define_cpu_exception_list!(INTERRUPT_LIST,
    [USER_SOFTWARE_INTERRUPT = 0, Interrupt],
    [SUPERVISOR_SOFTWARE_INTERRUPT = 1, Interrupt],
    [RESERVED_INTR_2 = 2, Reserved],
    [RESERVED_INTR_3 = 3, Reserved],
    [USER_TIMER_INTERRUPT = 4, Interrupt],
    [SUPERVISOR_TIMER_INTERRUPT = 5, Interrupt],
    [RESERVED_INTR_6 = 6, Reserved],
    [RESERVED_INTR_7 = 7, Reserved],
    [USER_EXTERNAL_INTERRUPT = 8, Interrupt],
    [SUPERVISOR_EXTERNAL_INTERRUPT = 9, Interrupt],
);

define_cpu_exception_list!(FAULT_LIST,
    [INSTRUCTION_ADDRESS_MISALIGNED = 0, Fault],
    [INSTRUCTION_ACCESS_FAULT = 1, Fault],
    [ILLEGAL_INSTRUCTION = 2, Fault],
    [BREAKPOINT = 3, Fault],
    [RESERVED_FAULT_4 = 4, Reserved],
    [LOAD_ACCESS_FAULT = 5, Fault],
    [AMO_ADDRESS_MISALIGNED = 6, Fault],
    [STORE_AMO_ACCESS_FAULT = 7, Fault],
    [ENVIRONMENT_CALL_FROM = 8, Fault],
    [RESERVED_FAULT_9 = 9, Reserved],
    [RESERVED_FAULT_10 = 10, Reserved],
    [RESERVED_FAULT_11 = 11, Reserved],
    [INSTRUCTION_PAGE_FAULT = 12, Fault],
    [LOAD_PAGE_FAULT = 13, Fault],
    [RESERVED_FAULT_14 = 14, Reserved],
    [STORE_AMO_PAGE_FAULT = 15, Fault],
);