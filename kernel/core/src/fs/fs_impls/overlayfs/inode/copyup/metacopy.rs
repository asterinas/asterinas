// SPDX-License-Identifier: MPL-2.0

//! The metacopy record: the value this mount writes and the validation every reader applies.
//!
//! A metacopy upper carries metadata only; its data stays in the first lower object. The record
//! itself is validated here and nowhere else, so `Metacopy` never reaches the generic positive-value
//! probe ([`OverlayXattrType::is_positive_on`]).
//!
//! The layout aligns with Linux: a four-byte header (`version`, `len`, `flags`, `digest_algo`)
//! followed by the digest, where `len` covers the header plus the used digest bytes.

use crate::{
    fs::{
        fs_impls::overlayfs::inode::OverlayXattrType,
        vfs::{path::Dentry, xattr::XattrNamespace},
    },
    prelude::*,
};

/// The v0 metacopy record: version 0, length 4, no flags, no digest.
///
/// The four bytes are `version`, `len`, `flags` and `digest_algo`. This mount's writer
/// emits this four-byte form; Linux stores the same record as an empty value, and both
/// kernels read both forms.
pub(super) const METACOPY_V0_RECORD: [u8; 4] = [0, 4, 0, 0];

/// The largest metacopy record this mount reads: the 4-byte header plus a 64-byte digest.
///
/// The minimum is the header alone; the bounded read rejects anything longer.
const METACOPY_MAX_BYTES: usize = 68;

/// Returns whether `real_dentry` carries a valid metadata-only record.
///
/// A zero-length value counts as the version-0 record. A value shorter than the header,
/// of another version, or whose length field disagrees with the bytes written is `EIO`.
/// An oversized value arrives as `ERANGE`.
pub(in super::super) fn has_metacopy(
    real_dentry: &Dentry,
    namespace: XattrNamespace,
) -> Result<bool> {
    if !real_dentry.inode().type_().is_regular_file() {
        return Ok(false);
    }
    let mut value = [0u8; METACOPY_MAX_BYTES];
    let written =
        match OverlayXattrType::Metacopy.get_value_from(real_dentry, namespace, &mut value) {
            Ok(written) => written,
            Err(err) if matches!(err.error(), Errno::ENODATA | Errno::EOPNOTSUPP) => {
                return Ok(false);
            }
            Err(err) if namespace == XattrNamespace::User && err.error() == Errno::EACCES => {
                return Ok(false);
            }
            // An oversized record arrives as ERANGE from the bounded buffer.
            Err(err) => return Err(err),
        };
    if written == 0 {
        return Ok(true);
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
    Ok(true)
}
