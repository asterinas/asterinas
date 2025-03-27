// SPDX-License-Identifier: MPL-2.0

use super::NLMSG_ALIGN;
use crate::prelude::*;

/// Aligns the `reader` to [`NLMSG_ALIGN`] by skipping some bytes.
///
/// If this method returns success, this method will return the number of skipped bytes.
pub fn align_reader(reader: &mut VmReader) -> Result<usize> {
    let align_offset = {
        let cursor = reader.cursor();
        cursor.align_offset(NLMSG_ALIGN)
    };

    if reader.remain() < align_offset {
        return_errno_with_message!(Errno::EFAULT, "the reader cannot be aligned");
    }

    reader.skip(align_offset);

    Ok(align_offset)
}

/// Aligns the `writer` to [`NLMSG_ALIGN`] by skipping some bytes.
///
/// If this method returns success, this method will return the number of skipped bytes.
pub fn align_writer(writer: &mut VmWriter) -> Result<usize> {
    let align_offset = {
        let cursor = writer.cursor();
        cursor.align_offset(NLMSG_ALIGN)
    };

    if writer.avail() < align_offset {
        return_errno_with_message!(Errno::EFAULT, "the writer cannot be aligned");
    }

    writer.skip(align_offset);

    Ok(align_offset)
}
