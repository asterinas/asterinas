// SPDX-License-Identifier: MPL-2.0

#![feature(generic_const_exprs)]
#![allow(incomplete_features)]
#![no_std]
#![deny(unsafe_code)]

//! An implementation of the global physical memory frame allocator for
//! [OSTD](https://crates.io/crates/ostd) based kernels.
//!
//! # Background
//!
//! `OSTD` has provided a page allocator interface, namely [`GlobalFrameAllocator`]
//! and [`global_frame_allocator`] procedure macro, allowing users to plug in
//! their own frame allocator into the kernel safely. You can refer to the
//! [`ostd::mm::frame::allocator`] module for detailed introduction.
//!
//! # Introduction
//!
//! This crate is an implementation of a scalable and efficient global frame
//! allocator based on the buddy system. It is by default shipped with OSDK
//! for users that don't have special requirements on the frame allocator.
//!
//! [`GlobalFrameAllocator`]: ostd::mm::GlobalFrameAllocator
//! [`global_frame_allocator`]: ostd::global_frame_allocator

pub(crate) mod allocator;
pub(crate) mod chunk;
pub(crate) mod set;

pub use allocator::{load_total_free_size, FrameAllocator};
