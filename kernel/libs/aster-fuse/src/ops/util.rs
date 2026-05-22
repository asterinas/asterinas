// SPDX-License-Identifier: MPL-2.0

//! Shared encoding and decoding helpers for FUSE operation implementations.

use ostd::mm::{Infallible, VmReader, VmWriter};
use ostd_pod::Pod;

use crate::{FuseError, FuseResult};

/// The trailing NULL byte required by FUSE request names.
pub(super) const NAME_TERMINATOR: &[u8] = &[0];

pub(super) fn name_body_len(prefix_len: usize, name: &str) -> usize {
    prefix_len
        .saturating_add(name.len())
        .saturating_add(NAME_TERMINATOR.len())
}

pub(super) fn read_bytes(reader: &mut VmReader<'_, Infallible>, dst: &mut [u8]) -> FuseResult<()> {
    if reader.remain() < dst.len() {
        return Err(FuseError::BufferTooSmall);
    }
    reader.read(&mut VmWriter::from(dst));
    Ok(())
}

pub(super) fn read_payload<T: Pod>(
    payload_len: usize,
    reader: &mut VmReader<'_, Infallible>,
) -> FuseResult<T> {
    if payload_len < size_of::<T>() {
        return Err(FuseError::MalformedResponse);
    }

    Ok(reader.read_val().unwrap())
}
