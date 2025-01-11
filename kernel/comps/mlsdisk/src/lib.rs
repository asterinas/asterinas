// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![deny(unsafe_code)]
#![feature(let_chains)]
#![feature(negative_impls)]
#![feature(slice_as_chunks)]
#![allow(dead_code, unused_imports)]

mod error;
mod layers;
mod os;
mod prelude;
mod tx;
mod util;

extern crate alloc;

pub use self::{
    error::{Errno, Error},
    layers::{
        bio::{BlockId, BlockSet, Buf, BufMut, BufRef, BLOCK_SIZE},
        disk::SwornDisk,
    },
    os::{Aead, AeadIv, AeadKey, AeadMac, Rng},
    util::{Aead as _, RandomInit, Rng as _},
};
