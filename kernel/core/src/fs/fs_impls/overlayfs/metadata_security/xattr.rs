// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(overlayfs)]
//! The xattr classification policy and delegation entries.
//!
//! This module classifies xattr names into public, private, reserved, and
//! escaped classes, admits every metadata_security xattr entry through the
//! permission pipeline, delegates real work under the creator-credential
//! scope, and filters overlay-private names from caller-visible results.
//!
//! # Classification
//!
//! A `trusted.overlay.`/`user.overlay.` name is `Private` when its suffix is
//! a known overlay record and `Reserved` otherwise; a `overlay.overlay.`
//! nesting-prefixed name is `Escaped`; everything else is `Public`.
//!
//! # Entry contract
//!
//! The **admission gate** is the [`OverlayInode::check_permission`] check;
//! every entry runs its classification or admission gate before any side
//! effect and forwards under the creator-credential scope through
//! `delegate_to_real`; `list_xattr` streams the underlying names through
//! [`XattrPolicy::filter_private_names`] so no private record reaches
//! the caller.
//!
//! # References
//!
//! - <https://elixir.bootlin.com/linux/v6.16.9/source/fs/overlayfs/overlayfs.h#L42-L54>
//! - <https://elixir.bootlin.com/linux/v6.16.9/source/fs/overlayfs/readdir.c#L614-L656>
//! - <https://elixir.bootlin.com/linux/v6.16.9/source/fs/overlayfs/util.c#L904-L917>
//! - <https://elixir.bootlin.com/linux/v6.16.9/source/fs/overlayfs/xattrs.c#L84-L96>

use crate::{
    fs::{
        file::Permission,
        fs_impls::overlayfs::{AccessType, inode::OverlayInode, superblock::OverlayFs},
        vfs::{
            inode::Inode,
            xattr::{XATTR_LIST_MAX_LEN, XattrName, XattrNamespace, XattrSetFlags},
        },
    },
    prelude::*,
};

/// The public/private/reserved/escaped classification policy.
///
/// Carries no state — classification is a pure function of the name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in overlayfs) struct XattrPolicy;

/// The four-way classification result of an xattr full name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum XattrClass {
    /// A user.*/system.*/security.*/trusted.* (non-overlay) name: delegate to
    /// the real authority.
    Public,
    /// A known Overlay-private record (suffix in `OVERLAY_PRIVATE_SUFFIXES`);
    /// classified by suffix; filtered from listing; refused through the generic
    /// path.
    Private,
    /// A `overlay.overlay.` nesting-prefixed name (refused and filtered).
    Escaped,
    /// An `overlay.*`-family name not in the known private table:
    /// policy-refused, never auto-promoted to `Public`.
    Reserved,
}

/// Known overlay-private record suffixes.
///
/// Suffix presence makes a name `Private`;
/// any other `trusted.overlay.`/`user.overlay.` suffix is `Reserved`.
const OVERLAY_PRIVATE_SUFFIXES: &[&str] = &[
    "opaque", "whiteout", "redirect", "origin", "impure", "nlink", "upper", "uuid", "metacopy",
    "protattr",
];

const TRUSTED_OVERLAY_PREFIX: &str = "trusted.overlay.";

const USER_OVERLAY_PREFIX: &str = "user.overlay.";

/// The persisted overlay UUID record name.
pub(in overlayfs) const TRUSTED_OVERLAY_UUID: &str = "trusted.overlay.uuid";

/// Returns the parsed [`XattrName`] for the overlay UUID record.
pub(in overlayfs) fn uuid_xattr_name() -> Result<XattrName<'static>> {
    XattrName::try_from_full_name(TRUSTED_OVERLAY_UUID)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid overlay uuid xattr name"))
}

/// The durable lower-source origin record name.
pub(in overlayfs) const ORIGIN_XATTR_FULL_NAME: &str = "trusted.overlay.origin";

/// Returns the parsed [`XattrName`] for the overlay origin record.
pub(in overlayfs) fn origin_xattr_name() -> Result<XattrName<'static>> {
    XattrName::try_from_full_name(ORIGIN_XATTR_FULL_NAME)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid overlay origin xattr name"))
}

/// The one-level nesting-escape prefix of a lower-overlay name.
const ESCAPED_OVERLAY_PREFIX: &str = "overlay.overlay.";

/// The opaque marker name; the clear-empty recipe writes it from here.
pub(in overlayfs) const OPAQUE_XATTR_FULL_NAME: &str = "trusted.overlay.opaque";

/// The opaque marker value; the reader requires the first byte `b'y'`.
pub(super) const OPAQUE_MARKER_VALUE: &[u8] = b"y";

/// The whiteout recipe consumes this name.
pub(in overlayfs) const WHITEOUT_XATTR_FULL_NAME: &str = "trusted.overlay.whiteout";

/// The whiteout marker value; the reader requires the first byte `b'y'`.
pub(in overlayfs) const WHITEOUT_MARKER_VALUE: &[u8] = b"y";

/// The impure marker is only ever read/written/cleared through the internal
/// [`XattrPolicy`] interface.
pub(super) const IMPURE_XATTR_FULL_NAME: &str = "trusted.overlay.impure";

/// The impure marker value; the reader is presence-based.
pub(super) const IMPURE_MARKER_VALUE: &[u8] = b"y";

/// The xattr-copy failure policy of the shared xattr copy
/// ([`XattrPolicy::copy_eligible_xattrs`]): strict aborts on a denied
/// source read or temp write, best-effort warns and skips; the transient
/// list/read race (`ENODATA`/`ERANGE`) always degrades to a skip.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in overlayfs) enum XattrCopyPolicy {
    /// Best-effort source reads and temp writes: EVERY xattr-copy error — a
    /// denied source read or temp write (`EACCES`/`EPERM`), the transient
    /// list/read race (`ENODATA`/`ERANGE`), and resource/I-O failures
    /// (`ENOSPC`/`EIO`, ...) alike — degrades to warn-and-skip and the
    /// operation continues. Used by the clear-empty recipe (`ClearEmpty`
    /// path), whose source directory is about to be deleted: a pure rmdir
    /// must never abort on the xattr fidelity copy.
    BestEffort,
    /// Strict source reads and temp writes: a denied source read or a
    /// temp-write error (`EACCES`/`EPERM`, `ENOSPC`, `EIO`, ...) propagates
    /// and fails the copy — the copy-up baseline — so no
    /// `security.*`/`trusted.*` xattr is ever silently dropped. Only the
    /// transient list/read race (`ENODATA`/`ERANGE`) degrades to a skip.
    Strict,
}

fn is_skippable_source_error(err: &Error, policy: XattrCopyPolicy) -> bool {
    policy == XattrCopyPolicy::BestEffort || matches!(err.error(), Errno::ENODATA | Errno::ERANGE)
}

/// The read semantics of an overlay marker xattr.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in overlayfs) enum MarkerReadSemantics {
    /// Presence probe (impure): any successful read counts as present, and
    /// `ERANGE` still counts as present.
    Presence,
    /// Value probe (whiteout/opaque): exactly one byte `b'y'` counts.
    ValueY,
}

impl XattrPolicy {
    /// Classifies an xattr full name into the four-way
    /// `Public`/`Private`/`Escaped`/`Reserved` classes.
    pub(super) fn classify(&self, full_name: &str) -> XattrClass {
        if let Some(suffix) = full_name
            .strip_prefix(TRUSTED_OVERLAY_PREFIX)
            .or_else(|| full_name.strip_prefix(USER_OVERLAY_PREFIX))
        {
            if OVERLAY_PRIVATE_SUFFIXES.contains(&suffix) {
                XattrClass::Private
            } else {
                XattrClass::Reserved
            }
        } else if full_name.starts_with(ESCAPED_OVERLAY_PREFIX) {
            XattrClass::Escaped
        } else {
            XattrClass::Public
        }
    }

    /// Returns whether `full_name` is an overlay-private xattr name.
    ///
    /// `true` exactly for the `Private`/`Escaped`/`Reserved` classes — the
    /// same name set the copy-time predicate excluded. This is the copy-time
    /// boundary filter; no duplicated predicate survives.
    pub(in overlayfs) fn is_private(&self, full_name: &str) -> bool {
        !matches!(self.classify(full_name), XattrClass::Public)
    }

    /// Streams the null-terminated raw name list, skipping every
    /// overlay-private name; with a zero-capacity `list_writer` (the
    /// `listxattr(path, NULL, 0)` size probe) it reports the total filtered
    /// size without writing.
    pub(super) fn filter_private_names(
        &self,
        raw_list: &[u8],
        list_writer: &mut VmWriter,
    ) -> Result<usize> {
        let mut bytes_written = 0;
        for name_bytes in raw_list.split(|&byte| byte == 0) {
            if name_bytes.is_empty() {
                continue;
            }
            let is_private =
                core::str::from_utf8(name_bytes).is_ok_and(|name| self.is_private(name));
            if is_private {
                continue;
            }
            let entry_len = name_bytes.len() + 1;
            if list_writer.avail() == 0 {
                bytes_written += entry_len;
                continue;
            }
            if entry_len > list_writer.avail() {
                return_errno_with_message!(
                    Errno::ERANGE,
                    "the xattr list buffer is too small for the filtered list"
                );
            }
            list_writer.write_fallible(&mut VmReader::from(name_bytes))?;
            list_writer.write_val(&0u8)?;
            bytes_written += entry_len;
        }
        Ok(bytes_written)
    }

    /// Copies the eligible public xattrs of `source` onto `temp`
    /// (copy-up / clear-empty) in the `User`/`Trusted`/`Security` namespaces,
    /// filtering overlay-private names.
    ///
    /// No creator-credential scope is currently available,
    /// so source reads run under the caller's credentials;
    /// a denied read therefore propagates as an error
    /// instead of silently dropping `security.*`/`trusted.*` xattrs.
    pub(in overlayfs) fn copy_eligible_xattrs(
        &self,
        source: &Arc<dyn Inode>,
        temp: &Arc<dyn Inode>,
        policy: XattrCopyPolicy,
    ) -> Result<()> {
        for namespace in [
            XattrNamespace::User,
            XattrNamespace::Trusted,
            XattrNamespace::Security,
        ] {
            let names = match Self::list_xattr_names(source, namespace) {
                Ok(names) => names,
                Err(err) if is_skippable_source_error(&err, policy) => {
                    warn!(
                        "overlay xattr copy: source xattr list unavailable for {:?}; \
                         skipping this namespace: {:?}",
                        namespace, err
                    );
                    continue;
                }
                Err(err) => return Err(err),
            };
            for full_name in names
                .split(|&byte| byte == 0)
                .filter(|name| !name.is_empty())
            {
                let Ok(full_name) = core::str::from_utf8(full_name) else {
                    // The VFS `XattrName` is UTF-8 text; a non-UTF-8 list
                    // entry cannot be represented and is skipped.
                    continue;
                };
                if self.is_private(full_name) {
                    continue;
                }
                // The parsed `XattrName` is reused for the value read and the
                // temp write.
                let Some(name) = XattrName::try_from_full_name(full_name) else {
                    warn!(
                        "overlay xattr copy: skipping unparsable xattr name: {}",
                        full_name
                    );
                    continue;
                };
                if name.namespace() != namespace {
                    continue;
                }
                let value = match Self::read_xattr_value(source, &name) {
                    Ok(value) => value,
                    Err(err) if is_skippable_source_error(&err, policy) => {
                        warn!("overlay xattr copy: skipping {}: {:?}", full_name, err);
                        continue;
                    }
                    Err(err) => return Err(err),
                };
                let mut reader = VmReader::from(value.as_slice()).to_fallible();
                match temp.set_xattr(name, &mut reader, XattrSetFlags::CREATE_OR_REPLACE) {
                    Err(err) if policy == XattrCopyPolicy::BestEffort => {
                        warn!(
                            "overlay xattr copy: skipping {} on temp: {:?}",
                            full_name, err
                        );
                        continue;
                    }
                    result => result?,
                }
            }
        }
        Ok(())
    }

    /// Lists the xattr names of one namespace on `source`, probing the
    /// required size with a zero-capacity writer before materializing the
    /// list; a size change between the two calls surfaces as `ERANGE`.
    fn list_xattr_names(source: &Arc<dyn Inode>, namespace: XattrNamespace) -> Result<Vec<u8>> {
        let mut probe = VmWriter::from(&mut [] as &mut [u8]).to_fallible();
        let list_len = source.list_xattr(namespace, &mut probe)?;
        let mut names = vec![0u8; list_len];
        let mut list_writer = VmWriter::from(names.as_mut_slice()).to_fallible();
        let written = source.list_xattr(namespace, &mut list_writer)?;
        names.truncate(written);
        Ok(names)
    }

    /// Reads one xattr value from `source`, probing the required size with a
    /// zero-capacity writer before materializing the value.
    fn read_xattr_value(source: &Arc<dyn Inode>, name: &XattrName<'_>) -> Result<Vec<u8>> {
        let reborrow_fn = || match XattrName::try_from_full_name(name.full_name()) {
            Some(name) => name,
            None => unreachable!("the copy loop validated this xattr name"),
        };
        let mut probe = VmWriter::from(&mut [] as &mut [u8]).to_fallible();
        let value_len = source.get_xattr(reborrow_fn(), &mut probe)?;
        let mut value = vec![0u8; value_len];
        let mut value_writer = VmWriter::from(value.as_mut_slice()).to_fallible();
        let written = source.get_xattr(reborrow_fn(), &mut value_writer)?;
        value.truncate(written);
        Ok(value)
    }

    /// Parses the impure marker's full name — the shared parse of the three
    /// marker methods.
    fn impure_marker_name() -> Result<XattrName<'static>> {
        XattrName::try_from_full_name(IMPURE_XATTR_FULL_NAME).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "invalid overlay impure marker xattr name")
        })
    }

    /// Parses the opaque marker's full name — the shared parse of every
    /// opaque-marker write/read site.
    pub(in overlayfs) fn opaque_marker_name() -> Result<XattrName<'static>> {
        XattrName::try_from_full_name(OPAQUE_XATTR_FULL_NAME).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "invalid overlay opaque marker xattr name")
        })
    }

    /// Parses the whiteout marker's full name — the shared parse of every
    /// whiteout-marker write/read site.
    pub(in overlayfs) fn whiteout_marker_name() -> Result<XattrName<'static>> {
        XattrName::try_from_full_name(WHITEOUT_XATTR_FULL_NAME).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "invalid overlay whiteout marker xattr name")
        })
    }

    /// Reads one overlay marker xattr under `semantics`.
    ///
    /// `ENODATA`/`EOPNOTSUPP` mean "marker absent"; `ERANGE` follows the
    /// semantics (present for `Presence`, absent for `ValueY`); other errors
    /// propagate unchanged.
    pub(in overlayfs) fn has_marker(
        &self,
        real_inode: &Arc<dyn Inode>,
        name: XattrName<'static>,
        semantics: MarkerReadSemantics,
    ) -> Result<bool> {
        let mut value = [0u8; 1];
        let mut writer = VmWriter::from(value.as_mut_slice()).to_fallible();
        match real_inode.get_xattr(name, &mut writer) {
            Ok(written) => match semantics {
                MarkerReadSemantics::Presence => Ok(true),
                MarkerReadSemantics::ValueY => Ok(written == 1 && value[0] == b'y'),
            },
            Err(err) if err.error() == Errno::ERANGE => {
                Ok(matches!(semantics, MarkerReadSemantics::Presence))
            }
            Err(err) if matches!(err.error(), Errno::ENODATA | Errno::EOPNOTSUPP) => Ok(false),
            Err(err) => Err(err),
        }
    }

    /// Returns whether `real_dir` carries the persisted impure marker.
    ///
    /// Presence probe on the real upper directory: the marker is interpreted
    /// by presence, not by value.
    pub(super) fn has_impure_marker(&self, real_dir: &Arc<dyn Inode>) -> Result<bool> {
        self.has_marker(
            real_dir,
            Self::impure_marker_name()?,
            MarkerReadSemantics::Presence,
        )
    }

    /// Persists the impure marker on the real upper directory `real_dir`.
    ///
    /// The internal write goes directly through the underlying inode — never
    /// through the user-facing `OverlayInode` xattr entries, whose `Private`
    /// refusal surface is untouched.
    pub(in overlayfs) fn set_impure_marker(&self, real_dir: &Arc<dyn Inode>) -> Result<()> {
        if self.has_impure_marker(real_dir)? {
            return Ok(());
        }
        debug_assert!(
            self.is_private(IMPURE_XATTR_FULL_NAME),
            "the impure marker name must classify as an overlay-private record"
        );
        let name = Self::impure_marker_name()?;
        let mut marker_reader = VmReader::from(IMPURE_MARKER_VALUE).to_fallible();
        real_dir.set_xattr(name, &mut marker_reader, XattrSetFlags::CREATE_OR_REPLACE)
    }

    /// Removes the impure marker from the real upper directory `real_dir`.
    /// Absence is already the cleared state, so clearing is idempotent.
    pub(super) fn clear_impure_marker(&self, real_dir: &Arc<dyn Inode>) -> Result<()> {
        let name = Self::impure_marker_name()?;
        match real_dir.remove_xattr(name) {
            Ok(()) => Ok(()),
            Err(err) if err.error() == Errno::ENODATA => Ok(()),
            Err(err) => Err(err),
        }
    }
}

impl OverlayFs {
    /// Writes the opaque marker onto `target` after the private-xattr
    /// capability gate; `unsupported_message` is the caller-scoped static
    /// `EOPNOTSUPP` message (the two call sites keep their distinct text).
    pub(in overlayfs) fn set_opaque_marker(
        &self,
        target: &Arc<dyn Inode>,
        unsupported_message: &'static str,
    ) -> Result<()> {
        let can_store_private_xattr = self
            .policy
            .upper_capabilities()
            .is_some_and(|caps| caps.can_store_private_xattr());
        if !can_store_private_xattr {
            return Err(Error::with_message(Errno::EOPNOTSUPP, unsupported_message));
        }
        let marker_name = XattrPolicy::opaque_marker_name()?;
        let mut marker_reader = VmReader::from(OPAQUE_MARKER_VALUE).to_fallible();
        target.set_xattr(
            marker_name,
            &mut marker_reader,
            XattrSetFlags::CREATE_OR_REPLACE,
        )
    }
}

impl OverlayInode {
    /// Refreshes the persisted impure marker after mutation,
    /// clearing it when no visible child keeps a non-empty lower stack.
    ///
    /// The clear is valid only under the immutable-lower premise:
    /// the lower stack is one the overlay never writes.
    /// Mounts with `default_permissions` do not implement this premise yet.
    ///
    /// A residual check-use race with an external lower writer
    /// cannot be closed by an overlay lock, so it is deliberately not handled.
    pub(in overlayfs) fn refresh_impure_marker(&self) -> Result<()> {
        // Upper-present gate: the marker lives only on real upper
        // directories; a lower-only directory cannot carry one.
        let facts = self.facts_snapshot();
        let Some(upper_real) = facts.upper.as_ref() else {
            return Ok(());
        };
        let fs = self.fs_arc()?;
        let xattr_policy = &fs.xattr_policy;
        if !xattr_policy.has_impure_marker(upper_real.real_inode())? {
            return Ok(());
        }
        self.ensure_readdir_index(&facts)?;
        let children: Vec<Arc<OverlayInode>> = {
            let index = self.readdir_index.as_ref().ok_or_else(|| {
                Error::with_message(Errno::ENOTDIR, "the overlay inode is not a directory")
            })?;
            index.lock().visible_inodes()
        };
        for child in &children {
            if !child.facts_snapshot().lowers.is_empty() {
                return Ok(());
            }
        }
        xattr_policy.clear_impure_marker(upper_real.real_inode())
    }

    /// Ensures `name` classifies as `Public` (the classification-refusal
    /// guard shared by the generic xattr entries): a non-`Public` name is
    /// refused BEFORE any admission side effect, with each entry's own
    /// refusal error (`EOPNOTSUPP` for `get_xattr`; `EPERM` for
    /// `set_xattr`/`remove_xattr`).
    fn ensure_public_xattr(&self, name: &XattrName, refusal: (Errno, &'static str)) -> Result<()> {
        if matches!(
            self.fs_arc()?.xattr_policy.classify(name.full_name()),
            XattrClass::Public
        ) {
            return Ok(());
        }
        Err(Error::with_message(refusal.0, refusal.1))
    }

    pub(in overlayfs) fn get_xattr_impl(
        &self,
        name: XattrName,
        value_writer: &mut VmWriter,
    ) -> Result<usize> {
        self.ensure_public_xattr(
            &name,
            (
                Errno::EOPNOTSUPP,
                "the overlay-private xattr is not exposed through the generic get path",
            ),
        )?;
        self.check_permission(AccessType::ReadOnly, Permission::MAY_READ)?;
        self.delegate_to_real(|real| real.get_xattr(name, value_writer))
    }

    pub(in overlayfs) fn set_xattr_impl(
        &self,
        name: XattrName,
        value_reader: &mut VmReader,
        flags: XattrSetFlags,
    ) -> Result<()> {
        self.ensure_public_xattr(
            &name,
            (
                Errno::EPERM,
                "overlay-private records cannot be forged through the generic set path",
            ),
        )?;
        self.check_permission(AccessType::Mutating, Permission::MAY_WRITE)?;
        self.delegate_to_real(|real| real.set_xattr(name, value_reader, flags))
    }

    // TODO: the `Permission::MAY_ACCESS` read-class demand is currently a
    // no-op placeholder because the DAC block does not evaluate `MAY_ACCESS`
    // yet. Remove this placeholder once `check_local_permission` evaluates
    // `MAY_ACCESS` for the `ReadOnly` class.
    pub(in overlayfs) fn list_xattr_impl(
        &self,
        namespace: XattrNamespace,
        list_writer: &mut VmWriter,
    ) -> Result<usize> {
        self.check_permission(AccessType::ReadOnly, Permission::MAY_ACCESS)?;
        self.delegate_to_real(|real| {
            let mut raw_list = vec![0u8; XATTR_LIST_MAX_LEN];
            let mut raw_writer = VmWriter::from(&mut raw_list[..]).to_fallible();
            let list_len = real.list_xattr(namespace, &mut raw_writer)?;
            let fs = self.fs_arc()?;
            fs.xattr_policy
                .filter_private_names(&raw_list[..list_len], list_writer)
        })
    }

    pub(in overlayfs) fn remove_xattr_impl(&self, name: XattrName) -> Result<()> {
        self.ensure_public_xattr(
            &name,
            (
                Errno::EPERM,
                "overlay-private records cannot be removed through the generic path",
            ),
        )?;
        self.check_permission(AccessType::Mutating, Permission::MAY_WRITE)?;
        self.delegate_to_real(|real| real.remove_xattr(name))
    }
}
