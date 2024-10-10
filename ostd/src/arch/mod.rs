// SPDX-License-Identifier: MPL-2.0

//! Platform-specific code.
//!
//! Each architecture that Asterinas supports may contain a submodule here.

use cfg_if::cfg_if;

cfg_if! {
    if #[cfg(target_arch = "riscv64")] {
        pub mod riscv;
        pub use self::riscv::*;
    } else if #[cfg(target_arch = "x86_64")] {
        pub mod x86;
        pub use self::x86::*;
    }
}
