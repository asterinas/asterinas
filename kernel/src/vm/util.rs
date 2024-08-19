// SPDX-License-Identifier: MPL-2.0

use ostd::mm::{Frame, FrameAllocOptions};

use crate::prelude::*;

/// Creates a new `Frame` and initializes it with the contents of the `src`.
pub fn duplicate_frame(src: &Frame) -> Result<Frame> {
    let new_frame = FrameAllocOptions::new(1).uninit(true).alloc_single()?;
    new_frame.copy_from(src);
    Ok(new_frame)
}
