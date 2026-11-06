// SPDX-License-Identifier: MPL-2.0

//! The redirect record: its codec, the per-inode accessor, and the value a move must record.
//!
//! A redirect names the lower entry an upper entry's content lives under. [`read_redirect`] is the
//! single reader; [`OverlayInode::redirect_record`] applies it to the receiver's own upper entry,
//! and [`OverlayInode::redirect_value_for`] composes the value one move writes. Which form that
//! value takes is [`OverlayInode::redirect_form`] alone.

use super::{OverlayInode, xattr::OverlayXattrType};
use crate::{
    fs::{
        fs_impls::overlayfs::fs::OverlayFs,
        vfs::{inode::Inode, path::Dentry, xattr::XattrNamespace},
    },
    prelude::*,
};

/// The redirect value limit in bytes, excluding any terminating NUL.
const REDIRECT_MAX_BYTES: usize = 256;

/// Returns the validated redirect value on `real_dentry`, or `None` when the record is absent or
/// zero-length.
pub(super) fn read_redirect(
    real_dentry: &Dentry,
    namespace: XattrNamespace,
) -> Result<Option<String>> {
    let mut value = [0u8; REDIRECT_MAX_BYTES + 1];
    let written =
        match OverlayXattrType::Redirect.get_value_from(real_dentry, namespace, &mut value) {
            Ok(written) => written,
            Err(err) if matches!(err.error(), Errno::ENODATA | Errno::EOPNOTSUPP) => {
                return Ok(None);
            }
            Err(err) if err.error() == Errno::ERANGE => {
                return_errno_with_message!(Errno::EINVAL, "the redirect record is too long");
            }
            Err(err) => return Err(err),
        };
    if written == 0 {
        return Ok(None);
    }
    if written > REDIRECT_MAX_BYTES {
        return_errno_with_message!(Errno::EINVAL, "the redirect record is too long");
    }
    let value = decode_redirect(&value[..written])?;
    Ok(Some(String::from(value)))
}

/// Validates one redirect value and returns it as a borrowed string.
fn decode_redirect(value: &[u8]) -> Result<&str> {
    if value.contains(&0) {
        return_errno_with_message!(Errno::EINVAL, "the redirect record contains a NUL byte");
    }
    let Ok(value) = core::str::from_utf8(value) else {
        return_errno_with_message!(Errno::EINVAL, "the redirect record is not valid UTF-8");
    };
    if let Some(absolute_path) = value.strip_prefix('/') {
        if absolute_path.is_empty() || absolute_path.ends_with('/') {
            return_errno_with_message!(Errno::EINVAL, "the absolute redirect path is malformed");
        }
        if absolute_path
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
        {
            return_errno_with_message!(Errno::EINVAL, "the absolute redirect path is malformed");
        }
    } else if value.contains('/') || value == "." || value == ".." {
        return_errno_with_message!(Errno::EINVAL, "the relative redirect name is malformed");
    }
    Ok(value)
}

/// The form a move records a redirect value in.
///
/// A relative value carries the name the object still has in the lower layer; an absolute value is
/// the object's path from the overlay root, which holds whichever of its names it is reached by.
/// Which form a move uses is [`OverlayInode::redirect_form`]'s decision, except for a hard link,
/// which names [`RedirectForm::Absolute`] directly because both of its names share one record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RedirectForm {
    /// Names the object by the name it holds in the lower layer.
    Relative,
    /// Names the object by its path from the overlay root.
    Absolute,
}

impl OverlayInode {
    /// Returns this object's redirect record, read from its upper entry alone.
    fn redirect_record(&self, namespace: XattrNamespace) -> Result<Option<String>> {
        let Some(upper) = self.upper.get() else {
            return Ok(None);
        };
        read_redirect(upper.dentry(), namespace)
    }

    /// Returns the redirect value a move of this object must record, or `None` when none is needed.
    pub(super) fn redirect_value_for(
        &self,
        fs: &OverlayFs,
        name: &str,
        parent_dentry: &Dentry,
        form: RedirectForm,
    ) -> Result<Option<String>> {
        let namespace = fs.policy().xattr_namespace();
        let own_record = self.redirect_record(namespace)?;
        // An existing record is reused as it stands: a relative request accepts any value, while an
        // absolute request only accepts an already-absolute one, which is path-independent. A
        // relative record therefore gets upgraded below.
        if let Some(existing) = &own_record
            && (matches!(form, RedirectForm::Relative) || existing.starts_with('/'))
        {
            return Ok(None);
        }
        if matches!(form, RedirectForm::Relative) {
            if name.len() > REDIRECT_MAX_BYTES {
                return_errno_with_message!(
                    Errno::EXDEV,
                    "the redirect path for the moved object exceeds the 256-byte limit"
                );
            }
            return Ok(Some(String::from(name)));
        }

        // The absolute value is the moved object's path from the overlay root, with each
        // level's redirect replacing that level's disk name when one exists.
        let mut components = vec![String::from(own_record.as_deref().unwrap_or(name))];
        let mut prefix: Option<String> = None;

        #[expect(
            unused_assignments,
            reason = "the loop always overwrites this placeholder before reading it"
        )]
        let mut owned_ancestor: Option<Arc<Dentry>> = None;
        let mut ancestor: Option<&Dentry> = Some(parent_dentry);
        while prefix.is_none() {
            let Some(dentry) = ancestor else {
                break;
            };
            let Some(parent) = dentry.parent() else {
                // The overlay root's name is not part of the value.
                break;
            };
            let ancestor_inode =
                dentry
                    .inode()
                    .downcast_ref::<OverlayInode>()
                    .ok_or_else(|| {
                        Error::with_message(
                            Errno::EIO,
                            "the redirect path left the overlay namespace",
                        )
                    })?;
            let record = ancestor_inode.redirect_record(namespace)?;
            let disk_name = dentry.name();
            let component = record.as_deref().unwrap_or(disk_name.as_str());
            if component.starts_with('/') {
                prefix = Some(String::from(component));
                break;
            }
            components.push(String::from(component));
            owned_ancestor = Some(parent);
            ancestor = owned_ancestor.as_deref();
        }

        let mut value = prefix.unwrap_or_default();
        for component in components.iter().rev() {
            let added = component.len().checked_add(1).ok_or_else(|| {
                Error::with_message(
                    Errno::EXDEV,
                    "the redirect path for the moved object exceeds the 256-byte limit",
                )
            })?;
            let new_len = value.len().checked_add(added).ok_or_else(|| {
                Error::with_message(
                    Errno::EXDEV,
                    "the redirect path for the moved object exceeds the 256-byte limit",
                )
            })?;
            if new_len > REDIRECT_MAX_BYTES {
                return_errno_with_message!(
                    Errno::EXDEV,
                    "the redirect path for the moved object exceeds the 256-byte limit"
                );
            }
            value.push('/');
            value.push_str(component);
        }
        Ok(Some(value))
    }

    /// Returns the form of the redirect value a move of this object must record, given whether the
    /// move crosses parent directories.
    pub(super) fn redirect_form(&self, is_cross_parent: bool) -> Result<RedirectForm> {
        // The lower probe is not short-circuited when is_cross_parent already decides the form, so
        // a failing lower metadata read is still reported instead of being hidden by the form.
        let mut is_absolute = is_cross_parent;
        if !self.type_().is_directory()
            && let Some(lower) = self.lowers.first()
        {
            is_absolute |= lower.real_inode().metadata()?.nr_hard_links > 1;
        }
        Ok(if is_absolute {
            RedirectForm::Absolute
        } else {
            RedirectForm::Relative
        })
    }
}
