// SPDX-License-Identifier: MPL-2.0

use ostd::mm::{Frame, FrameAllocOptions, UntypedMem, UntypedMeta};

use crate::prelude::*;

/// Creates a new `Frame<dyn UntypedMeta>` and initializes it with the contents of the `src`.
pub fn duplicate_frame(src: &Frame<dyn UntypedMeta>) -> Result<Frame<()>> {
    let new_frame = FrameAllocOptions::new().zero_init(false).alloc_frame()?;
    new_frame.writer().write(&mut src.reader());
    Ok(new_frame)
}
