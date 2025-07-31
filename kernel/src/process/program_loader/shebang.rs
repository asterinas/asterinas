// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

/// Tries to parse a buffer as a shebang line.
///
/// If the buffer starts with `#!` and its header is a valid shebang sequence,
/// then the function returns `Ok(Some(parts))`, where `parts` is a `Vec` that
/// contains the path of and the arguments for the interpreter.
///
/// If the buffer starts with `#!` but some error occurs while parsing the
/// file, then `Err(_)` is returned. If the buffer does not start with `#!`,
/// then `Ok(None)` is returned.
pub fn parse_shebang_line(file_first_page: &[u8]) -> Result<Option<Vec<CString>>> {
    if !file_first_page.starts_with(b"#!") || !file_first_page.contains(&b'\n') {
        // The file is not a shebang.
        return Ok(None);
    }
    let Some(first_line_len) = file_first_page.iter().position(|&c| c == b'\n') else {
        return_errno_with_message!(Errno::ENAMETOOLONG, "The shebang line is too long");
    };
    // Skip `#!`.
    let shebang_header = &file_first_page[2..first_line_len];
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
