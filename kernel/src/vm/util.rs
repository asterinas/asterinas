// SPDX-License-Identifier: MPL-2.0

use ostd::mm::{FrameAllocOptions, UntypedFrame};

use crate::prelude::*;

/// Creates a new `UntypedFrame` and initializes it with the contents of the `src`.
pub fn duplicate_frame(src: &UntypedFrame) -> Result<UntypedFrame> {
    let new_frame = FrameAllocOptions::new(1).uninit(true).alloc_single()?;
    new_frame.copy_from(src);
    Ok(new_frame)
}
