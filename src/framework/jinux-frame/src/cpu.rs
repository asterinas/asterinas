//! CPU.

cfg_if::cfg_if! {
    if #[cfg(feature="x86_64")]{

        pub use crate::arch::x86::cpu::CpuContext;
        pub use crate::arch::x86::cpu::TrapInformation;
        pub use crate::arch::x86::cpu::GpRegs;
        pub use crate::arch::x86::cpu::FpRegs;

        /// Returns the number of CPUs.
        pub fn num_cpus() -> u32 {
            crate::arch::x86::cpu::num_cpus()
        }

        /// Returns the ID of this CPU.
        pub fn this_cpu() -> u32 {
            crate::arch::x86::cpu::this_cpu()
        }

    }
}
