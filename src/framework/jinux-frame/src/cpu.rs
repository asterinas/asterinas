//! CPU.

cfg_if::cfg_if! {
    if #[cfg(feature="x86_64")]{
        pub use crate::arch::x86::cpu::CpuContext;
        pub use crate::arch::x86::cpu::FpRegs;
        pub use crate::arch::x86::cpu::TrapInformation;
        pub use trapframe::GeneralRegs;
        pub use crate::arch::x86::cpu::num_cpus;
        pub use crate::arch::x86::cpu::this_cpu;
    }
}
