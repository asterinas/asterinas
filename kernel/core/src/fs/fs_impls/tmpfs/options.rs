// SPDX-License-Identifier: MPL-2.0

//! Mount option parsing for tmpfs.
//!
//! The `mode`, `uid`, and `gid` options set the mode and owner of the root directory.

use crate::{
    fs::{file::InodeMode, utils},
    prelude::*,
    process::{Gid, Uid},
};

/// The mode of the root directory when the `mode` mount option is absent.
//
// Reference: <https://elixir.bootlin.com/linux/v6.8.12/source/mm/shmem.c#L4585>
pub(super) const DEFAULT_ROOT_MODE: InodeMode = InodeMode::from_bits_truncate(0o1777);

/// The tmpfs mount options that set the mode and owner of the root directory.
pub(super) struct MountOptions {
    pub(super) root_mode: InodeMode,
    pub(super) root_uid: Uid,
    pub(super) root_gid: Gid,
}

impl MountOptions {
    /// Parses the comma-separated tmpfs mount options.
    ///
    /// Without the `mode`, `uid`, and `gid` options, the root directory has mode 1777
    /// and is owned by the filesystem user and group IDs of the current thread.
    //
    // Reference: <https://elixir.bootlin.com/linux/v6.8.12/source/mm/shmem.c#L3937-L3967>
    pub(super) fn parse(args: Option<&str>) -> Result<Self> {
        let (root_uid, root_gid) = utils::current_fs_ids();
        let mut options = Self {
            root_mode: DEFAULT_ROOT_MODE,
            root_uid,
            root_gid,
        };
        let Some(args) = args else {
            return Ok(options);
        };

        for option in args.split(',') {
            let (key, value) = match option.split_once('=') {
                Some((key, value)) => (key, Some(value)),
                None => (option, None),
            };
            match key {
                "mode" => {
                    let mode = parse_u32(value, Radix::Octal)? & 0o7777;
                    options.root_mode = InodeMode::from_bits_truncate(mode as u16);
                }
                "uid" => {
                    let uid = Uid::new(parse_u32(value, Radix::Auto)?);
                    if uid == Uid::INVALID {
                        return Err(errno!(EINVAL, "invalid tmpfs `uid` value"));
                    }
                    options.root_uid = uid;
                }
                "gid" => {
                    let gid = Gid::new(parse_u32(value, Radix::Auto)?);
                    if gid == Gid::INVALID {
                        return Err(errno!(EINVAL, "invalid tmpfs `gid` value"));
                    }
                    options.root_gid = gid;
                }
                // TODO: Support `size`, `nr_inodes`, and the other tmpfs mount options,
                // and reject unknown ones.
                _ => {}
            }
        }

        Ok(options)
    }
}

/// The radix of a numeric mount option value.
#[derive(Clone, Copy)]
enum Radix {
    /// Octal.
    Octal,
    /// Hexadecimal with a `0x` prefix, octal with a `0` prefix, and decimal otherwise.
    Auto,
}

/// Parses a numeric mount option value as Linux's `kstrtouint` does.
fn parse_u32(value: Option<&str>, radix: Radix) -> Result<u32> {
    let value = value.ok_or_else(|| errno!(EINVAL, "the tmpfs mount option requires a value"))?;
    let value = value.strip_suffix('\n').unwrap_or(value);
    let value = value.strip_prefix('+').unwrap_or(value);

    let (digits, base) = match radix {
        Radix::Octal => (value, 8),
        Radix::Auto => {
            if let Some(hex_digits) = value
                .strip_prefix("0x")
                .or_else(|| value.strip_prefix("0X"))
            {
                (hex_digits, 16)
            } else if value.starts_with('0') {
                (value, 8)
            } else {
                (value, 10)
            }
        }
    };

    // `from_str_radix` accepts a sign before the digits, but `kstrtouint` accepts
    // only the single `+` stripped above.
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(base)) {
        return Err(errno!(EINVAL, "invalid tmpfs mount option value"));
    }
    u32::from_str_radix(digits, base)
        .map_err(|_| errno!(EINVAL, "tmpfs mount option value out of range"))
}
