#[cfg(target_arch = "x86_64")]
mod x86_64;
#[cfg(target_arch = "x86_64")]
pub use x86_64::*;

#[cfg(target_arch = "x86")]
mod x86;
#[cfg(target_arch = "x86")]
pub use x86::*;

#[cfg(target_arch = "riscv64")]
mod riscv64;
#[cfg(target_arch = "riscv64")]
pub use riscv64::*;

#[cfg(target_arch = "riscv32")]
mod riscv32;
#[cfg(target_arch = "riscv32")]
pub use riscv32::*;

#[cfg(target_arch = "aarch64")]
mod aarch64;
#[cfg(target_arch = "aarch64")]
pub use aarch64::*;

#[cfg(target_arch = "loongarch64")]
mod loongarch64;
#[cfg(target_arch = "loongarch64")]
pub use loongarch64::*;

#[cfg(not(any(
    target_arch = "x86_64",
    target_arch = "x86",
    target_arch = "riscv64",
    target_arch = "riscv32",
    target_arch = "aarch64",
    target_arch = "loongarch64"
)))]
compile_error!("Current architecture is not supported");

// CFI directives cannot be used if neither debuginfo nor panic=unwind is enabled.
// We don't have an easy way to check the former, so just check based on panic strategy.
#[cfg(panic = "abort")]
macro_rules! maybe_cfi {
    ($x: literal) => {
        ""
    };
}

#[cfg(panic = "unwind")]
macro_rules! maybe_cfi {
    ($x: literal) => {
        $x
    };
}

pub(crate) use maybe_cfi;
