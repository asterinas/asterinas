// SPDX-License-Identifier: MPL-2.0

//! LZ4 input preparation and complete-output decoding.

use io_util::IoError;

/// Removes LZ4 zero-padding without modifying the cached raw input.
pub(in super::super) fn prepare_input(input: &[u8]) -> Result<&[u8], IoError> {
    let payload_start_bytes = input
        .iter()
        .position(|byte| *byte != 0)
        .ok_or(IoError::Failed)?;
    Ok(&input[payload_start_bytes..])
}

/// Decodes one LZ4 block that must fill the complete output buffer.
pub(in super::super) fn decompress(input: &[u8], output: &mut [u8]) -> Result<(), IoError> {
    let decoded_len_bytes =
        lz4_flex::block::decompress_into(input, output).map_err(|_| IoError::Failed)?;
    if decoded_len_bytes != output.len() {
        return Err(IoError::Failed);
    }
    Ok(())
}
