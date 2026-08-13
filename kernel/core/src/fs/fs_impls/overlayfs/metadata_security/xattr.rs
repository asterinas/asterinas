// SPDX-License-Identifier: MPL-2.0

//! The xattr classification policy and delegation entries.
//!
//! This module hosts the `metadata_security/xattr.rs` surface: the payload-less
//! [`OverlayXattrPolicy`] policy value (stateless; owned once by the
//! `OverlayFs::xattr_policy` field, initialized in `mount/build.rs`), its
//! [`XattrClass`] classification result, the module-private known private-name
//! table and prefix constants, the `classify`/`is_private`/
//! `filter_private_names` methods, the shared classification-aware xattr copy
//! [`OverlayXattrPolicy::copy_eligible_xattrs`] (the single
//! classification-aware copy loop of the overlayfs tree, shared by the
//! copy-up and clear-empty paths through the [`XattrCopyPolicy`] failure
//! policy), and the four `Inode`-trait xattr entries
//! (`get_xattr`/`set_xattr`/`list_xattr`/`remove_xattr`).
//!
//! Classification semantics: a name under the `trusted.overlay.`/
//! `user.overlay.` private namespace is `Private` when its suffix is a known
//! overlay record (the `OVERLAY_PRIVATE_SUFFIXES` table) and `Reserved`
//! otherwise (an `overlay.*`-family name is policy-refused and never
//! auto-promoted to `Public`); a `overlay.overlay.` nesting-prefixed name is
//! `Escaped` (refused/filtered, never un-escaped); everything else is `Public`
//! and delegates to the real authority. `is_private` is the judgment method:
//! it returns `true` exactly for the `Private`/`Escaped`/`Reserved` classes —
//! the same name set the copy-time predicate excluded — so copy behavior is
//! preserved while the classification authority lives here.
//!
//! Entry contract: the classification stage runs **before**
//! `check_permission` for `set_xattr`/`remove_xattr` so a non-`Public` name is
//! refused with no promotion side effect (`EOPNOTSUPP` for `get_xattr`;
//! `EPERM` for `set_xattr`/`remove_xattr`); `list_xattr` streams the underlying raw
//! name list through [`OverlayXattrPolicy::filter_private_names`] so no
//! private record ever reaches the caller. `get_xattr`/`list_xattr` carry the
//! read-class admission demand (`AccessType::ReadOnly`): `get_xattr` uses
//! `Permission::MAY_READ` (a real read-DAC gate) and `list_xattr` uses
//! `Permission::MAY_ACCESS` (a placeholder — the current DAC block does not
//! evaluate `MAY_ACCESS`, so the list gate is a no-op until DAC supports it);
//! `set_xattr`/`remove_xattr` use the uniform mutating shape
//! (`AccessType::Mutating`, `Permission::MAY_WRITE`): the EROFS gate runs in
//! the local stage and the copy-up runs in the entry `check_permission`
//! (both independent of the `default_permissions` skip), then forward under
//! the creator-credential scope
//! through the single private delegation helper `delegate_to_real` (defined
//! in `mod.rs` so the three sibling files share it).
//!
//! Lock contract: this module acquires no Overlay lock. The classification
//! stage and the admission surface are lock-free local stages; the only lock
//! progression is inside the authority promotion (`ensure_upper_authority`,
//! consumed between the two permission stages), and no Overlay lock is ever
//! held across an underlying xattr callback. The underlying xattr ops
//! self-evaluate under the creator-credential scope (ext2/ramfs evidence), so
//! the explicit real stage is a benign double evaluation kept for
//! security-gate independence.

use crate::{
    fs::{
        file::Permission,
        fs_impls::overlayfs::{
            AccessType, projection::OverlayInode, readdir_index::ReaddirIndexEntry,
        },
        vfs::{
            inode::Inode,
            xattr::{XATTR_LIST_MAX_LEN, XattrName, XattrNamespace, XattrSetFlags},
        },
    },
    prelude::*,
};

/// The public/private/escaped classification policy.
///
/// Stateless: the private-namespace prefixes (`TRUSTED_OVERLAY_PREFIX`,
/// `USER_OVERLAY_PREFIX`) and the escape prefix (`ESCAPED_OVERLAY_PREFIX`) are
/// module-private consts; userxattr namespace selection and escaping are
/// future features that would add state here — no field is pre-baked.
/// Immutable; owned once by `OverlayFs::xattr_policy`; no lock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::fs::fs_impls::overlayfs) struct OverlayXattrPolicy;

/// The payload-less four-way classification result of an xattr full name.
///
/// Payload-less: the four-way classification is the semantic and every entry
/// branches only `Public`-vs-rest; the per-record classification of the
/// removed `OverlayPrivateXattr` payload enum is preserved as the
/// module-private `OVERLAY_PRIVATE_SUFFIXES` table and the comment below,
/// because no consumer reads the payload — each suffix only needs its
/// classification here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::fs::fs_impls::overlayfs) enum XattrClass {
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

/// Known overlay-private record suffixes and where each is handled:
///   whiteout, opaque -> directory mutation
///   redirect         -> not yet handled here
///   origin, upper    -> inode association tracking
///   impure           -> metadata/xattr handling (this module)
///   nlink            -> copy-up link counting
///   uuid             -> mount UUID persistence
///   metacopy         -> not yet handled here
///   protattr         -> file attributes (not yet handled here)
const OVERLAY_PRIVATE_SUFFIXES: &[&str] = &[
    "opaque", "whiteout", "redirect", "origin", "impure", "nlink", "upper", "uuid", "metacopy",
    "protattr",
];

/// The private-namespace prefix of the persisted overlay records.
const TRUSTED_OVERLAY_PREFIX: &str = "trusted.overlay.";

/// The user-namespace mirror of the persisted overlay records.
const USER_OVERLAY_PREFIX: &str = "user.overlay.";

/// The one-level nesting-escape prefix of a lower-overlay name.
const ESCAPED_OVERLAY_PREFIX: &str = "overlay.overlay.";

/// The xattr full name of the opaque-directory marker (Linux `OVL_XATTR_OPAQUE`).
///
/// This module declares the marker name (the `opaque` suffix is a known
/// overlay-private record in `OVERLAY_PRIVATE_SUFFIXES`); the `dir/create.rs`
/// and `dir/remove.rs` recipes reference it instead of redeclaring it.
/// `projection/entry.rs` still carries its own copy of the name.
pub(in crate::fs::fs_impls::overlayfs) const OPAQUE_XATTR_FULL_NAME: &str =
    "trusted.overlay.opaque";

/// The opaque marker value (Linux writes `"y"`; the reader requires the first
/// byte `b'y'`).
pub(in crate::fs::fs_impls::overlayfs) const OPAQUE_MARKER_VALUE: &[u8] = b"y";

/// The xattr full name of the xattr-based whiteout marker (Linux
/// `OVL_XATTR_XWHITEOUT`).
///
/// Central declaration of the marker name: `dir/whiteout.rs` (the owning
/// operation) and `legacy_fs.rs` import it from here instead of redeclaring
/// it. `projection/entry.rs` still carries its own copy of the name.
pub(in crate::fs::fs_impls::overlayfs) const WHITEOUT_XATTR_FULL_NAME: &str =
    "trusted.overlay.whiteout";

/// The xattr full name of the impure-directory marker (Linux `OVL_XATTR_IMPURE`).
///
/// The `impure` suffix is already a known overlay-private record
/// (`OVERLAY_PRIVATE_SUFFIXES`), so the name/value pair lives here as the
/// single declaration; the marker is only ever read/written/cleared through
/// the internal [`OverlayXattrPolicy`] interface — never through the user-facing
/// xattr entries.
pub(in crate::fs::fs_impls::overlayfs) const IMPURE_XATTR_FULL_NAME: &str =
    "trusted.overlay.impure";

/// The impure marker value (Linux writes `"y"`; the reader is presence-based).
pub(in crate::fs::fs_impls::overlayfs) const IMPURE_MARKER_VALUE: &[u8] = b"y";

/// The xattr-copy failure policy of the shared xattr copy
/// ([`OverlayXattrPolicy::copy_eligible_xattrs`]) — a small closed enum
/// (never a bool) selecting whether a source read or temp write that fails
/// (a denied access, a resource/I-O error, ...) aborts the copy (strict) or
/// degrades to warn-and-skip (best-effort).
///
/// The variants are named for the behavior they select; the two copy paths
/// map onto them as follows:
/// - [`XattrCopyPolicy::BestEffort`] (clear-empty path): the source is the
///   displaced upper directory of a clear-empty exchange, which is being
///   deleted, so its xattrs are moot — every copy error degrades and the
///   non-owner rmdir succeeds.
/// - [`XattrCopyPolicy::Strict`] (copy-up path): the copied object is
///   persisted, so a denied source read must fail the copy-up rather than
///   silently drop `security.*`/`trusted.*` metadata.
///
/// The list/read race (`ENODATA`/`ERANGE` — a concurrent xattr mutation
/// between the probe and the materialized read) always degrades to a skip;
/// it is a transient mutation, not a failure, under both policies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::fs::fs_impls::overlayfs) enum XattrCopyPolicy {
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

/// Returns whether a SOURCE-read error of the shared xattr copy is skippable
/// under `policy` (the single source-read skip decision of
/// [`OverlayXattrPolicy::copy_eligible_xattrs`]; one helper for the
/// namespace-list and value-read arms, so the predicate can never diverge).
///
/// The list/read race (`ENODATA`/`ERANGE` — the source list/value changed
/// between the probe and the materialized read) is a transient mutation and
/// skips under BOTH policies. Under the best-effort
/// [`XattrCopyPolicy::BestEffort`] (the `ClearEmpty` path) EVERY source-read
/// error — a permission-denied read (`EACCES`/`EPERM`, the no-op
/// creator-credential mechanism) as well as resource/I-O failures
/// (`ENOSPC`/`EIO`, ...) — degrades to warn-and-skip, because the doomed
/// source directory's xattr fidelity copy must never abort the pure rmdir.
/// Under the strict [`XattrCopyPolicy::Strict`] (`CopyUp` path) only the
/// transient race skips and every other error propagates (no silent
/// `security.*`/`trusted.*` loss on the persisted copy). The exact predicate
/// is executed by the two source-error arms of `copy_eligible_xattrs` inside
/// this module. A free pure function rather than an owner-local method: it
/// reads no `self` state (pure `errno` × `policy` decision), so an unused
/// `&self` receiver would add noise without an invariant to guard.
fn is_skippable_source_error(err: &Error, policy: XattrCopyPolicy) -> bool {
    policy == XattrCopyPolicy::BestEffort || matches!(err.error(), Errno::ENODATA | Errno::ERANGE)
}

impl OverlayXattrPolicy {
    /// Classifies an xattr full name into the four-way
    /// `Public`/`Private`/`Escaped`/`Reserved` classes.
    ///
    /// Pure and lock-free: a name under the private-namespace prefixes whose
    /// suffix is in `OVERLAY_PRIVATE_SUFFIXES` is `Private` and any other
    /// `overlay.*`-family name is `Reserved` (never auto-promoted to
    /// `Public`); a `overlay.overlay.` nesting-prefixed name is `Escaped`;
    /// everything else is `Public` and delegates to the real authority.
    /// `classify` is part of the public/private/escaped classification that
    /// sibling modules consume through `OverlayFs::xattr_policy()`.
    pub(in crate::fs::fs_impls::overlayfs) fn classify(&self, full_name: &str) -> XattrClass {
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
    /// The judgment method: `!matches!(self.classify(full_name),
    /// XattrClass::Public)` — `true` exactly for the `Private`/`Escaped`/
    /// `Reserved` classes, the same name set the copy-time predicate excluded.
    /// This is the copy-time boundary filter; no duplicated predicate
    /// survives.
    pub(in crate::fs::fs_impls::overlayfs) fn is_private(&self, full_name: &str) -> bool {
        !matches!(self.classify(full_name), XattrClass::Public)
    }

    /// Streams the null-terminated raw name list from the underlying listing,
    /// skipping every name with `is_private == true` and writing the
    /// survivors to `list_writer`.
    ///
    /// Returns the number of bytes written (each survivor is written with its
    /// trailing null byte); with a zero-capacity `list_writer` (the
    /// `listxattr(path, NULL, 0)` size probe) no byte is written and the
    /// returned length is the total filtered size. A writer that cannot fit
    /// the next survivor returns `ERANGE` (the caller's buffer is too small);
    /// the probe contract requires the zero-capacity call to succeed with the
    /// required length, never `EINVAL`. The intermediate raw list is bounded
    /// by `XATTR_LIST_MAX_LEN`: the underlying list always fits, so an
    /// oversized real list surfaces as the underlying `ERANGE` before any
    /// survivor is written. Invariant-preserving filter: a private record
    /// (`Private`/`Escaped`/`Reserved`) never leaks through the listing. A
    /// non-UTF-8 name cannot be an overlay-private record (all private names
    /// are ASCII), so it is forwarded unchanged rather than failing or
    /// leaking. Private to the `metadata_security` tree; only the `list_xattr`
    /// entry consumes it.
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
                // Size probe: accumulate the required length without writing
                // (the caller's `listxattr(path, NULL, 0)` probe must return
                // the total filtered size, not fail).
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

    /// Copies the eligible public xattrs of `source` onto `temp` (copy-up /
    /// clear-empty) — the single shared classification-aware xattr copy of
    /// the overlayfs tree.
    ///
    /// Enumerates the `User`, `Trusted`, and `Security` namespaces — the
    /// `System` namespace (`system.posix_acl_*`) is reserved for ACLs and
    /// stays excluded on every copy path — and filters overlay-private
    /// names through [`OverlayXattrPolicy::is_private`]. Every listed name is
    /// additionally namespace-filtered after parsing (a non-filtering backend
    /// can list names of other namespaces under one probe, e.g. `user.*`
    /// under a `Trusted` list or `system.posix_acl_*` under a `Security`
    /// list; such cross-namespace names are skipped, so `System` stays
    /// excluded and no duplicate is copied), and an unparsable name (an
    /// unknown namespace such as `lustre.*`) is skipped with a warning
    /// instead of hard-failing the copy. The clear-empty
    /// recipe writes the temp's own `trusted.overlay.opaque` marker
    /// explicitly, so copying the displaced upper dir's marker would
    /// double-mark and is excluded by the same rule.
    ///
    /// The failure policy is selected by the caller through the closed
    /// [`XattrCopyPolicy`] enum:
    /// - **Best-effort source reads** ([`XattrCopyPolicy::BestEffort`],
    ///   `ClearEmpty` path): the source is the displaced upper directory of
    ///   a clear-empty exchange, which is being deleted, so its xattrs are
    ///   moot. EVERY source-read error — a denied read (`EACCES`/`EPERM` on
    ///   the namespace list or the value) as well as resource/I-O failures
    ///   (`ENOSPC`/`EIO`, ...) — degrades to "warn + skip" and the operation
    ///   continues, restoring the success path for a non-owner rmdir
    ///   of an owner-only xattr-carrying directory and keeping a pure rmdir
    ///   independent of the doomed directory's xattr fidelity copy.
    /// - **Strict source reads** ([`XattrCopyPolicy::Strict`], `CopyUp`
    ///   path): the copied object is persisted, so `EACCES`/`EPERM` on the
    ///   source namespace list or the value read PROPAGATES and the copy-up
    ///   fails — the copy-up baseline — with NO silent
    ///   `security.*`/`trusted.*` loss.
    /// - **Race degradation (both policies):** a concurrent xattr mutation
    ///   between the probe and the materialized read surfaces as
    ///   `ENODATA`/`ERANGE` and degrades to "skip this xattr" (value read) or
    ///   "skip this namespace" (list probe), each with a `warn!`, never an
    ///   abort of the operation.
    /// - **Best-effort temp writes ([`XattrCopyPolicy::BestEffort`],
    ///   `ClearEmpty` path):** the temp `set_xattr` is part of the same
    ///   best-effort exchange — the displaced upper dir is being deleted and
    ///   the opaque temp is whiteouted — so ANY temp-write failure (a denied
    ///   write `EACCES`/`EPERM`, e.g. the temp's mode lacks owner-write for a
    ///   `user.*` xattr; the transient `ENODATA`/`ERANGE`; or a resource/I-O
    ///   error `ENOSPC`/`EIO`) degrades to "warn + skip this xattr" and the
    ///   clear-empty rmdir succeeds. Under the
    ///   strict [`XattrCopyPolicy::Strict`] a temp-write error still
    ///   propagates and fails the persisted copy.
    ///
    /// Known divergence: the underlying source reads run under the CALLER's
    /// credentials because `with_creator_credentials_fn` is a documented
    /// no-op mechanism; Linux copies under the creator's credentials and
    /// preserves these xattrs. The strict copy-up policy refuses the silent
    /// loss by propagating the denial — Linux's successful non-owner copy
    /// requires executing the source reads under the creator's credentials,
    /// which the VFS cannot do yet; until that is possible, the strict
    /// policy propagates the denial instead of silently dropping xattrs.
    /// Genuine xattr errors still hard-fail and abort the exchange
    /// before the rename; an unparsable list entry (a non-UTF-8 or
    /// unknown-namespace name) is skipped with a warning — it cannot be
    /// represented as a VFS `XattrName` — never hard-failed.
    pub(in crate::fs::fs_impls::overlayfs) fn copy_eligible_xattrs(
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
            // A source LIST error follows the selected policy: best-effort
            // degrades EVERY error to "skip this namespace's copy" with a
            // `warn!` (the clear-empty source is being deleted, so a pure
            // rmdir must never abort on the fidelity copy); strict
            // propagates every error except the list/read race (a persisted
            // copy must not lose the namespace). The list/read race
            // (`ENODATA`/`ERANGE` — the list grew between the probe and the
            // materialized read) degrades to "skip this namespace" under
            // BOTH policies (a transient mutation, never an abort).
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
                // The name is validated exactly once per copied xattr and the
                // validated `XattrName` is threaded through the value read and
                // the temp write — no re-validation of `full_name` and no
                // duplicated EINVAL error literal. An unparsable list entry
                // (an unknown namespace such as `lustre.*`) cannot be
                // represented as a VFS `XattrName`; it is skipped with a
                // warning (the "System/lustre excluded" copy intent) instead
                // of hard-failing the whole copy.
                let Some(name) = XattrName::try_from_full_name(full_name) else {
                    warn!(
                        "overlay xattr copy: skipping unparsable xattr name: {}",
                        full_name
                    );
                    continue;
                };
                // Explicit namespace filter: a non-filtering backend may list
                // names of other namespaces under this namespace's probe (e.g.
                // `user.*` under a `Trusted` list, or `system.posix_acl_*`
                // under a `Security` list). Only names whose parsed namespace
                // matches the probed one are copied — `System` stays excluded
                // on every copy path and no cross-namespace duplicate is
                // copied.
                if name.namespace() != namespace {
                    continue;
                }
                // Source value-read failures: the documented list/read race
                // (`ENODATA`/`ERANGE` — value removed or resized between the
                // probe and the materialized read) degrades to "skip this
                // xattr" under BOTH policies; under best-effort
                // ([`XattrCopyPolicy::BestEffort`], `ClearEmpty` path) EVERY
                // source value-read error — a denied read (`EACCES`/`EPERM`,
                // the no-op creator-credential mechanism in `mount/policy.rs`) as
                // well as resource/I-O failures (`ENOSPC`/`EIO`, ...) —
                // skips with a `warn!` (the clear-empty source being
                // deleted); strict propagates every error but the race (no
                // silent security-metadata loss on the persisted copy-up).
                let value = match Self::read_xattr_value(source, &name) {
                    Ok(value) => value,
                    Err(err) if is_skippable_source_error(&err, policy) => {
                        warn!("overlay xattr copy: skipping {}: {:?}", full_name, err);
                        continue;
                    }
                    Err(err) => return Err(err),
                };
                let mut reader = VmReader::from(value.as_slice()).to_fallible();
                // Best-effort temp writes: the displaced upper dir is being
                // deleted and the opaque temp is whiteouted, so ANY failed
                // temp `set_xattr` — a denied write (`EACCES`/`EPERM`, e.g.
                // the temp's mode lacks owner-write for a `user.*` xattr), the
                // transient race (`ENODATA`/`ERANGE`), or a resource/I-O
                // failure (`ENOSPC`/`EIO`, ...) — degrades to warn + skip
                // THIS xattr instead of aborting the whole clear-empty
                // exchange (a pure rmdir must never abort on the fidelity
                // copy). Strict keeps the copy-up baseline: the persisted
                // object must not lose metadata, so every temp-write error
                // still hard-fails.
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

    /// Lists the xattr names of one namespace on `source`.
    ///
    /// The VFS list convention is probed with a zero-capacity writer (returns
    /// the required size) and then materialized into an exactly sized buffer
    /// (ramfs/ext2 precedent; a size change between the two calls surfaces as
    /// `ERANGE`, which the caller treats as the documented list/read race and
    /// skips that namespace — never an abort). Invoked once per namespace
    /// (three times per copy call) from
    /// [`OverlayXattrPolicy::copy_eligible_xattrs`].
    fn list_xattr_names(source: &Arc<dyn Inode>, namespace: XattrNamespace) -> Result<Vec<u8>> {
        let mut probe = VmWriter::from(&mut [] as &mut [u8]).to_fallible();
        let list_len = source.list_xattr(namespace, &mut probe)?;
        let mut names = vec![0u8; list_len];
        let mut list_writer = VmWriter::from(names.as_mut_slice()).to_fallible();
        let written = source.list_xattr(namespace, &mut list_writer)?;
        names.truncate(written);
        Ok(names)
    }

    /// Reads one xattr value from `source`.
    ///
    /// The value length is probed with a zero-capacity writer and the value is
    /// then materialized into an exactly sized buffer. `XattrName` is not
    /// `Copy` and carries no `Clone` (VFS surface), so each `get_xattr`
    /// call takes its own owned view; both views are re-borrowed from the
    /// caller's already-validated name (validated exactly once in the copy
    /// loop; an unparsable list entry was already skipped with a warning
    /// there), so the helper carries no validation and no error site of its
    /// own. Invoked once per
    /// listed name (multiple times per copy call) from
    /// [`OverlayXattrPolicy::copy_eligible_xattrs`].
    fn read_xattr_value(source: &Arc<dyn Inode>, name: &XattrName<'_>) -> Result<Vec<u8>> {
        // `XattrName` is not `Copy`/`Clone`, so each `get_xattr` re-borrows a
        // thin owned view of the same full name. The copy loop validated
        // `name` exactly once; re-parsing the same full name cannot fail (the
        // recorded hard-invariant `unreachable!` precedent of the tree, never
        // `.unwrap()`/`.expect()`).
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
    /// marker methods (one name constant, one error literal).
    fn impure_marker_name() -> Result<XattrName<'static>> {
        XattrName::try_from_full_name(IMPURE_XATTR_FULL_NAME).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "invalid overlay impure marker xattr name")
        })
    }

    /// Returns whether `real_dir` carries the persisted impure marker.
    ///
    /// Presence probe on the real upper directory (Linux
    /// `ovl_cache_get_impure` read). The marker read is presence-based: an
    /// absent (`ENODATA`) or unsupported (`EOPNOTSUPP`) marker reads as "not
    /// impure", while a value longer than the 1-byte probe (`ERANGE`) still
    /// proves presence. Genuine xattr errors propagate.
    pub(in crate::fs::fs_impls::overlayfs) fn has_impure_marker(
        &self,
        real_dir: &Arc<dyn Inode>,
    ) -> Result<bool> {
        let name = Self::impure_marker_name()?;
        let mut value = [0u8; IMPURE_MARKER_VALUE.len()];
        let mut writer = VmWriter::from(value.as_mut_slice()).to_fallible();
        match real_dir.get_xattr(name, &mut writer) {
            Ok(_) => Ok(true),
            Err(err) if err.error() == Errno::ERANGE => Ok(true),
            Err(err) if err.error() == Errno::ENODATA || err.error() == Errno::EOPNOTSUPP => {
                Ok(false)
            }
            Err(err) => Err(err),
        }
    }

    /// Persists the impure marker on the real upper directory `real_dir`.
    ///
    /// Read-first idempotent: an already-marked directory is not written
    /// again (Linux `ovl_set_impure` no-op parity). The internal write goes
    /// directly through the underlying inode — never through the user-facing
    /// `OverlayInode` xattr entries, whose `Private` refusal surface is
    /// untouched.
    pub(in crate::fs::fs_impls::overlayfs) fn set_impure_marker(
        &self,
        real_dir: &Arc<dyn Inode>,
    ) -> Result<()> {
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
    ///
    /// Internal `remove_xattr`; an absent marker (`ENODATA`) reads as the
    /// already-cleared state and returns `Ok(())` (idempotent). Genuine xattr
    /// errors propagate.
    pub(in crate::fs::fs_impls::overlayfs) fn clear_impure_marker(
        &self,
        real_dir: &Arc<dyn Inode>,
    ) -> Result<()> {
        let name = Self::impure_marker_name()?;
        match real_dir.remove_xattr(name) {
            Ok(()) => Ok(()),
            Err(err) if err.error() == Errno::ENODATA => Ok(()),
            Err(err) => Err(err),
        }
    }
}

impl OverlayInode {
    /// Refreshes this directory's persisted impure marker against its current
    /// visible children.
    ///
    /// The lifecycle coordinator: a directory without a real upper cannot
    /// carry the marker (`Ok(())` no-op); a directory whose marker is absent
    /// has nothing to refresh; otherwise the index is ensured `Valid` under
    /// the caller's `DIR` transaction, the `Visible` child `Arc`s are cloned
    /// under a brief index `INODE` lock (released before any per-child facts
    /// snapshot or xattr call), and the marker is cleared when NO visible
    /// child keeps a non-empty lower stack (the purity predicate: a child
    /// keeps the marker iff its lower stack is non-empty; whiteout residue is
    /// never counted). Callers invoke it best-effort
    /// (warn-and-continue) after the underlying mutation has already
    /// committed, matching Linux `ovl_cache_get_impure`.
    ///
    /// Immutable-lower premise: the clear is valid only on a lower stack the
    /// overlay itself never writes. On mounts without `default_permissions`
    /// that guarantee already holds; on `default_permissions` mounts it is
    /// not yet implemented, so the marker clear is only safe once the
    /// permission path never writes to lower layers — until then that
    /// configuration keeps a documented limitation. External concurrent
    /// modification of the lower layers is an unsupported operation
    /// (documented). The residual check-use race — an external lower writer
    /// adding content between the children scan and the clear — is a known
    /// limitation recorded here: no overlay lock can close it (the writer is
    /// outside the kernel), and the defensive re-check (clear then re-read
    /// and re-scan) would only narrow, never close, the window while adding
    /// a second scan and marker write on a cold path, so it is deliberately
    /// not implemented.
    pub(in crate::fs::fs_impls::overlayfs) fn refresh_impure_marker(&self) -> Result<()> {
        // Upper-present gate: the marker lives only on real upper
        // directories; a lower-only directory cannot carry one (defensive
        // early return, never an unwrap).
        let facts = self.facts_snapshot();
        let Some(upper_real) = facts.upper() else {
            return Ok(());
        };
        let fs = self.fs_arc()?;
        let xattr_policy = fs.xattr_policy();
        if !xattr_policy.has_impure_marker(upper_real.real_inode())? {
            return Ok(());
        }
        // Purity scan under the caller-held `DIR` transaction: ensure the
        // index is `Valid`, then clone the `Visible` child `Arc`s under a
        // brief index `INODE` lock. No `INODE` guard is held across a child
        // `facts_snapshot` or the marker clear below (`DIR -> INODE` order).
        self.ensure_readdir_index(&facts)?;
        let children: Vec<Arc<OverlayInode>> = {
            let index = self.readdir_index().ok_or_else(|| {
                Error::with_message(Errno::ENOTDIR, "the overlay inode is not a directory")
            })?;
            let index = index.lock();
            index
                .entries
                .iter()
                .filter_map(|entry| match entry {
                    ReaddirIndexEntry::Visible { inode, .. } => Some(inode.clone()),
                    ReaddirIndexEntry::Tombstone { .. } => None,
                })
                .collect()
        };
        // Per-child lowers scan: any visible child with a non-empty lower
        // stack keeps the marker.
        for child in &children {
            if !child.facts_snapshot().lowers().is_empty() {
                return Ok(());
            }
        }
        xattr_policy.clear_impure_marker(upper_real.real_inode())
    }

    /// Ensures `name` classifies as `Public` (the classification-refusal
    /// guard shared by the three generic xattr entries).
    ///
    /// A non-`Public` overlay-private name is refused BEFORE any admission
    /// side effect. Each entry supplies its own refusal error: `EOPNOTSUPP`
    /// for `get_xattr` (Linux v4.10+ `ovl_xattr_get` semantics — the
    /// pre-v4.10 `ENODATA` is not returned) and `EPERM` for
    /// `set_xattr`/`remove_xattr` (a private record cannot be forged or
    /// removed through the generic path).
    fn ensure_public_xattr(&self, name: &XattrName, refusal: (Errno, &'static str)) -> Result<()> {
        if matches!(
            self.fs_arc()?.xattr_policy().classify(name.full_name()),
            XattrClass::Public
        ) {
            return Ok(());
        }
        Err(Error::with_message(refusal.0, refusal.1))
    }

    // Xattr get: classification refusal runs first (before the admission, so
    // no authority side effect ever starts for a private name); the refusal
    // returns `EOPNOTSUPP` for every non-`Public` name (Linux v4.10+
    // `ovl_xattr_get` semantics — the pre-v4.10 `ENODATA` is not returned),
    // then the read-DAC demand (`AccessType::ReadOnly`,
    // `Permission::MAY_READ`; namespace gating already ran in the syscall
    // layer), then a creator-credential forward to the current real
    // authority. The underlying `get_xattr` self-evaluates under the
    // creator-credential scope (ext2/ramfs evidence); the explicit real stage
    // inside `check_permission` is the benign double evaluation kept for
    // gate-independence.
    pub(in crate::fs::fs_impls::overlayfs) fn get_xattr_impl(
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

    // Xattr set: the classification stage runs BEFORE the mutating admission
    // so a non-`Public` name is refused with no promotion side effect, then
    // the uniform mutating shape (`AccessType::Mutating`,
    // `Permission::MAY_WRITE` — the EROFS gate runs in the local stage and
    // the copy-up runs in the entry `check_permission`, both independent of
    // the `default_permissions` skip), then a creator-credential forward.
    // The underlying `set_xattr` self-evaluates under the creator-credential
    // scope; the explicit real stage is the benign double evaluation.
    pub(in crate::fs::fs_impls::overlayfs) fn set_xattr_impl(
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

    // Xattr list: the read-class admission demand `Permission::MAY_ACCESS`
    // (the access bit required by the list semantics, matching the underlying
    // ext2/ramfs list self-evaluation), then the real listing into the
    // bounded `XATTR_LIST_MAX_LEN` intermediate, then the private-name filter
    // streaming pass so `Private`/`Escaped`/`Reserved` records never reach
    // the caller. The filtered length returned by `filter_private_names` is
    // the number of bytes written to `list_writer`.
    //
    // TODO: the current DAC block (VFS `inode.rs:573-640` / overlay
    // Projected-DAC) does not evaluate `MAY_ACCESS`, so this gate is a no-op
    // for now; it becomes effective only after DAC support for `MAY_ACCESS`
    // lands. The actual read-side constraint is carried by the `get`'s
    // `MAY_READ` gate plus the underlying list self-evaluation.
    pub(in crate::fs::fs_impls::overlayfs) fn list_xattr_impl(
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
            fs.xattr_policy()
                .filter_private_names(&raw_list[..list_len], list_writer)
        })
    }

    // Xattr remove: identical shape to `set_xattr` — classification refusal
    // (`EPERM`) before the mutating admission, so a non-`Public` name is
    // refused with no promotion side effect, then the uniform mutating shape
    // and a creator-credential forward to the current real authority.
    pub(in crate::fs::fs_impls::overlayfs) fn remove_xattr_impl(
        &self,
        name: XattrName,
    ) -> Result<()> {
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
