// SPDX-License-Identifier: MPL-2.0

//! The metacopy record: the value this mount writes and the validation every reader applies.
//!
//! A metacopy upper carries metadata only; its data stays in the first lower object. The record
//! itself is validated here and nowhere else, so `Metacopy` never reaches the generic positive-value
//! probe ([`OverlayXattrType::is_positive_on`]).
//!
//! The layout is Linux's `struct ovl_metacopy`, bounded by `OVL_METACOPY_MIN_SIZE` and
//! `OVL_METACOPY_MAX_SIZE` and initialized by `OVL_METACOPY_INIT`: `version`, `len` (the header
//! plus the used digest bytes), `flags`, `digest_algo`, then the digest.
//!
//! Reference:
//! <https://elixir.bootlin.com/linux/v6.18/source/fs/overlayfs/overlayfs.h#L165-L175>

use crate::{
    fs::{
        fs_impls::overlayfs::inode::OverlayXattrType,
        vfs::{path::Dentry, xattr::XattrNamespace},
    },
    prelude::*,
};

/// The v0 metacopy record: version 0, length 4, no flags, no digest.
///
/// The four bytes are `version`, `len`, `flags` and `digest_algo`, i.e. Linux's
/// `OVL_METACOPY_INIT` (the 4-byte form this mount's writer emits). Linux's
/// `ovl_set_metacopy_xattr()` stores the same record as an empty value; both kernels read both
/// forms.
///
/// Reference:
/// <https://elixir.bootlin.com/linux/v6.18/source/fs/overlayfs/util.c#L1277-L1287>
pub(super) const METACOPY_V0_RECORD: [u8; 4] = [0, 4, 0, 0];

/// The largest metacopy record this mount reads: the 4-byte header plus a 64-byte digest.
///
/// This is Linux's `OVL_METACOPY_MAX_SIZE`, of which `OVL_METACOPY_MIN_SIZE` is the header alone.
///
/// Reference:
/// <https://elixir.bootlin.com/linux/v6.18/source/fs/overlayfs/overlayfs.h#L165-L175>
const METACOPY_MAX_BYTES: usize = 68;

/// Reads and validates the metacopy record, returning its length when the record is present.
///
/// This applies Linux's `ovl_check_metacopy_xattr()` rules: a zero-length value is emulated as
/// `{len: 4}`, while a value shorter than 4 bytes, of another version, or whose `len` differs
/// from the bytes written is rejected with `EIO`.
///
/// Reference:
/// <https://elixir.bootlin.com/linux/v6.18/source/fs/overlayfs/util.c#L1223-L1268>
pub(in super::super) fn check_metacopy(
    real_dentry: &Dentry,
    namespace: XattrNamespace,
) -> Result<Option<u32>> {
    if !real_dentry.inode().type_().is_regular_file() {
        return Ok(None);
    }
    let mut value = [0u8; METACOPY_MAX_BYTES];
    let written =
        match OverlayXattrType::Metacopy.get_value_from(real_dentry, namespace, &mut value) {
            Ok(written) => written,
            Err(err) if matches!(err.error(), Errno::ENODATA | Errno::EOPNOTSUPP) => {
                return Ok(None);
            }
            Err(err) if namespace == XattrNamespace::User && err.error() == Errno::EACCES => {
                return Ok(None);
            }
            // An oversized record arrives as ERANGE from the bounded buffer.
            Err(err) => return Err(err),
        };
    if written == 0 {
        return Ok(Some(METACOPY_V0_RECORD.len() as u32));
    }
    if written < METACOPY_V0_RECORD.len() {
        return_errno_with_message!(Errno::EIO, "the metacopy record is too short");
    }
    if value[0] != METACOPY_V0_RECORD[0] {
        return_errno_with_message!(Errno::EIO, "the metacopy record has an unsupported version");
    }
    if value[1] as usize != written {
        return_errno_with_message!(Errno::EIO, "the metacopy record has an inconsistent length");
    }
    Ok(Some(written as u32))
}
