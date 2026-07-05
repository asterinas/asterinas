#![doc = include_str!("../README.md")]
// We use `non_exhaustive_omitted_patterns_lint` which is a nightly lint.
#![allow(unknown_lints)]
#![cfg_attr(
    any(
        feature = "personality",
        feature = "personality-dummy",
        feature = "panicking",
        feature = "panic-handler-dummy"
    ),
    allow(internal_features)
)]
#![cfg_attr(
    any(feature = "personality", feature = "personality-dummy"),
    feature(lang_items)
)]
#![cfg_attr(
    any(
        feature = "panicking",
        all(
            any(feature = "panic", feature = "panic-handler-dummy"),
            not(feature = "libc")
        )
    ),
    feature(core_intrinsics)
)]
#![cfg_attr(feature = "panic-handler", feature(thread_local))]
#![no_std]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "unwinder")]
mod unwinder;

#[cfg(all(feature = "unwinder", feature = "fde-custom"))]
pub use unwinder::custom_eh_frame_finder;

pub mod abi;

mod arch;
mod util;

#[cfg(feature = "print")]
pub mod print;

#[cfg(feature = "personality")]
mod personality;
#[cfg(all(not(feature = "personality"), feature = "personality-dummy"))]
mod personality_dummy;

#[cfg(feature = "panic")]
pub mod panic;
#[cfg(feature = "panicking")]
pub mod panicking;

#[cfg(feature = "panic-handler")]
mod panic_handler;
#[cfg(all(not(feature = "panic-handler"), feature = "panic-handler-dummy"))]
mod panic_handler_dummy;

#[cfg(feature = "system-alloc")]
mod system_alloc;
