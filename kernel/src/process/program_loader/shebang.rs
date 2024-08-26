// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

/// Try to parse a buffer as a shebang line.
///
/// If the buffer starts with `#!` and its header is a valid shebang sequence,
/// then the function returns `Ok(Some(parts))`,
/// where `parts` is a `Vec` that contains the path of and the arguments for the interpreter.
/// If the buffer starts with `#!` but some error occurs while parsing the file,
/// then `Err(_)` is returned.
/// If the buffer does not start with `#!`, then `Ok(None)` is returned.
pub fn parse_shebang_line(file_header_buffer: &[u8]) -> Result<Option<Vec<CString>>> {
    if !file_header_buffer.starts_with(b"#!") || !file_header_buffer.contains(&b'\n') {
        // the file is not a shebang
        return Ok(None);
    }
    let first_line_len = file_header_buffer.iter().position(|&c| c == b'\n').unwrap();
    // skip #!
    let shebang_header = &file_header_buffer[2..first_line_len];
    let mut shebang_argv = Vec::new();
    for arg in shebang_header.split(|&c| c == b' ') {
        let arg = CString::new(arg)?;
        shebang_argv.push(arg);
    }
    if shebang_argv.len() != 1 {
        return_errno_with_message!(
            Errno::EINVAL,
            "One and only one interpreter program should be specified"
        );
    }
    Ok(Some(shebang_argv))
}
