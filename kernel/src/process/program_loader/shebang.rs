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
        return_errno_with_message!(Errno::ENAMETOOLONG, "the shebang line is too long");
    };

    // Skip `#!`.
    let shebang_header = &file_first_page[2..first_line_len];
    let mut shebang_argv = Vec::new();
    for arg in shebang_header.split(|&c| c == b' ' || c == b'\t') {
        if arg.is_empty() {
            continue;
        }

        let arg = CString::new(arg).map_err(|_| {
            Error::with_message(Errno::ENOEXEC, "unexpected nul terminator is found")
        })?;
        shebang_argv.push(arg);
    }

    if shebang_argv.is_empty() {
        return_errno_with_message!(Errno::ENOEXEC, "no interpreter program is found");
    }

    Ok(Some(shebang_argv))
}

#[cfg(ktest)]
mod test {
    use alloc::{ffi::CString, vec};

    use ostd::prelude::*;

    use super::parse_shebang_line;

    #[ktest]
    fn parse_shebang_line_with_multiple_args() {
        const LINE1: &str = "#! /bin/bash -e\n";
        let res = parse_shebang_line(LINE1.as_bytes()).unwrap().unwrap();
        assert_eq!(
            res,
            vec![
                CString::new("/bin/bash").unwrap(),
                CString::new("-e").unwrap()
            ]
        );

        const LINE2: &str = "#!  /bin/env  python3  \n";
        let res = parse_shebang_line(LINE2.as_bytes()).unwrap().unwrap();
        assert_eq!(
            res,
            vec![
                CString::new("/bin/env").unwrap(),
                CString::new("python3").unwrap()
            ]
        );
    }
}
