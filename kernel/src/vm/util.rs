// SPDX-License-Identifier: MPL-2.0

use ostd::mm::{Frame, FrameAllocOptions, UFrame, UntypedMem};

use crate::prelude::*;

/// Creates a new `Frame<()>` and initializes it with the contents of the `src`.
///
/// Note that it only duplicates the contents not the metadata.
pub fn duplicate_frame(src: &UFrame) -> Result<Frame<()>> {
    let new_frame = FrameAllocOptions::new().zeroed(false).alloc_frame()?;
    new_frame.writer().write(&mut src.reader());
    Ok(new_frame)
}
