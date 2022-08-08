//! The framework part of KxOS.
#![no_std]
#![allow(dead_code)]
#![allow(unused_variables)]
#![feature(negative_impls)]

extern crate alloc;

pub mod cpu;
pub mod device;
mod error;
pub mod prelude;
pub mod task;
pub mod timer;
pub mod user;
mod util;
pub mod vm;

pub use self::error::Error;
