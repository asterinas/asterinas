// SPDX-License-Identifier: MPL-2.0

//! Control-device regression modules and shared ABI buffer fixtures.

use super::abi::{DM_IOCTL_HEADER_SIZE, OFF_DATA_SIZE, OFF_DATA_START, write_u32, write_version};
use crate::prelude::*;

mod control_abi_tests;
mod ioctl_params_tests;
mod management_tests;

fn test_buffer(size: usize) -> Vec<u8> {
    let mut buffer = vec![0u8; size];
    write_u32(&mut buffer, OFF_DATA_SIZE, size).unwrap();
    write_u32(&mut buffer, OFF_DATA_START, DM_IOCTL_HEADER_SIZE).unwrap();
    buffer
}

fn request(size: usize) -> Vec<u8> {
    let mut bytes = test_buffer(size);
    write_version(&mut bytes).unwrap();
    bytes
}

fn version() -> [u8; 12] {
    let mut bytes = [0; 12];
    write_version(&mut bytes).unwrap();
    bytes
}
