// SPDX-License-Identifier: MPL-2.0

//! CPU-related definitions.

pub mod cpu_local;

cfg_if::cfg_if! {
    if #[cfg(target_arch = "x86_64")]{
        pub use crate::arch::x86::cpu::*;
    }
}
