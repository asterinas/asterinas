// SPDX-License-Identifier: MPL-2.0

pub(crate) use alloc::{
    collections::VecDeque,
    string::String,
    sync::{Arc, Weak},
    vec,
    vec::Vec,
};
pub(crate) use core::fmt::Debug;

pub(crate) use bitflags::bitflags;

pub(crate) type Result<T> = core::result::Result<T, aster_nix::error::Error>;
