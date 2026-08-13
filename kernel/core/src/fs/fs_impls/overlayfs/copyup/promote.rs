// SPDX-License-Identifier: MPL-2.0

//! The copy-up winner body: object-kind promotion and upper publication.
//!
//! This module hosts the private winner body [`OverlayInode::promote`], its
//! object-kind recipe arms ([`OverlayInode::promote_regular_file`],
//! [`OverlayInode::promote_symlink`] and the inline `Dir` arm), the
//! metadata/xattr transfer steps ([`OverlayInode::transfer_metadata`],
//! [`OverlayInode::transfer_timestamps`], [`OverlayInode::copy_eligible_xattrs`]),
//! the ReconcilePending verification ([`OverlayInode::verify_upper_target`],
//! [`OverlayInode::upper_real_object`]), and the publication steps — the
//! shared atomic-rename tail ([`OverlayInode::publish_via_rename`]) and the
//! semantic authority publication ([`OverlayInode::publish_upper_authority`]).
//!
//! Lock contract: the trigger (trigger.rs) acquires the `CUL` guard
//! (`OverlayInode::copyup_transition`) and HOLDS it across this whole winner
//! body — the re-snapshot (waiter leg), the ReconcilePending scope
//! inspection (recovery), and the promotion recipe including the semantic
//! publication — so no concurrent winner can interleave between the
//! re-snapshot and the publication (the double copy-up TOCTOU is closed).
//! The winner reads the coordinate once under the guard and passes it into
//! [`OverlayInode::promote`]; every helper in this file consumes the passed
//! `publication_parent`/`name` instead of performing its own brief `CUL`
//! read, so the non-reentrant `ostd::sync::Mutex` is never re-acquired (the
//! guard is sleep-capable — promotion may BIO under it). The `INODE`-domain
//! facts snapshots stay brief and are never held across an underlying call.

use core::cmp::min;

use super::{
    coordination::{CopyUpPhase, CopyUpTransition},
    workdir::WorkdirTempRequest,
};
use crate::{
    fs::{
        file::{InodeType, StatusFlags},
        fs_impls::overlayfs::{
            metadata_security::xattr::XattrCopyPolicy,
            mount::{OverlayFs, RealPath},
            projection::{OverlayInode, OverlayObjectFacts, PositiveKind, RealObject},
        },
        vfs::{
            inode::{Inode, MknodType, RenameMode, SymbolicLink},
            path::Path,
        },
    },
    prelude::*,
};

/// The chunk size of the regular-file data stream during copy-up.
///
/// The lower file is streamed through one reused kernel buffer; the chunk
/// bounds each `read_at`/`write_at` pair so a short read still makes bounded
/// progress. A pure numeric local, not a named production type.
const COPY_CHUNK_SIZE: usize = 64 * 1024;

impl OverlayInode {
    /// Runs the winner promotion body for this object.
    ///
    /// Winner body: called by the trigger (trigger.rs) with the `CUL`
    /// arbitration guard **held** and the publication coordinate passed in —
    /// the trigger read the coordinate once, then holds `copyup_transition`
    /// across the re-snapshot, the ReconcilePending verification, and this
    /// whole recipe, so a concurrent winner can never interleave between the
    /// re-snapshot and the semantic publication (the double copy-up TOCTOU is
    /// closed). The object kind is derived from the topmost lower real object
    /// (`lowers[0].real_inode().type_()`) and dispatched internally (match
    /// arms; no recipe enum). The `CUL` guard is never re-acquired inside
    /// this body: every helper consumes the passed coordinate
    /// (`publication_parent`/`name`) instead of re-reading it, so the
    /// non-reentrant mutex cannot deadlock. The success path commits the
    /// phase to [`CopyUpPhase::Idle`] through the passed coordinate; the
    /// recipe arms classify failures (cleanup before publication vs
    /// `ReconcilePending` after publication).
    ///
    /// The coordinate contents are read by the trigger under the held guard;
    /// the ReconcilePending marker (recovery) is derived directly from
    /// `coordinate.phase` inside this body, under the held guard (no
    /// redundant bool crosses the trigger boundary): a pending reconcile
    /// means the upper entry at `(publication_parent, name)` must be verified
    /// before reuse.
    pub(super) fn promote(
        &self,
        publication_parent: &Arc<OverlayInode>,
        name: &str,
        coordinate: &mut CopyUpTransition,
    ) -> Result<()> {
        // 1) Idempotent upper fast path: a waiter may have completed the
        //    transition while this task waited for the arbitration guard (the
        //    trigger re-snapshots under the guard; this is the defensive
        //    re-check at the winner boundary) — a brief facts snapshot, no
        //    `CUL`.
        if self.facts_snapshot().upper().is_some() {
            return Ok(());
        }

        // 2) ReconcilePending verification (recovery) under the held `CUL`
        //    guard: the marker is derived directly from the coordinate phase
        //    (no redundant bool crosses the trigger boundary). The upper
        //    parent existence is resolved by the trigger's ancestor walk, so
        //    its real object is the upper directory. The verify helper
        //    consumes the passed `name` (no `CUL` re-read under the held
        //    guard).
        if coordinate.phase == CopyUpPhase::ReconcilePending {
            let upper_dir_path = publication_parent.upper_parent_path()?;
            self.verify_upper_target(&upper_dir_path, name)?;
        }

        // 3) Upper/workdir operations below run against the underlying
        //    filesystem's own locking; the `CUL` guard stays held across them
        //    by design (sleep-capable mutex).
        let upper_dir = publication_parent.select_real_inode();
        let upper_dir_path = publication_parent.upper_parent_path()?;
        let fs = self.fs_arc()?;
        // Impurity marker: every promoted object makes its publication
        // parent impure — persist the marker before the object-kind dispatch
        // and the physical upper commit (strict, pre-commit; read-first
        // idempotence makes an already-marked parent a no-op).
        fs.xattr_policy().set_impure_marker(&upper_dir)?;
        let lower = self.lower_source()?;
        let result = match lower.real_inode().type_() {
            InodeType::Dir => {
                // Directory copy-up: a private workdir temp directory,
                // metadata/xattr transfer, then atomic publication via
                // `RenameMode::Replace` (the `File`/`SymLink` recovery
                // discipline: a stale upper entry at the publication
                // coordinate, including a `ReconcilePending` residue, is
                // atomically replaced instead of failing `create` with
                // `EEXIST`). No children are copied.
                let mode = lower.real_inode().mode()?;
                let temp = fs.create_workdir_temp(
                    name,
                    &upper_dir_path,
                    WorkdirTempRequest::Create {
                        kind: InodeType::Dir,
                        mode,
                    },
                )?;
                let temp_kind = temp.kind();
                let (temp_name, temp) = temp.into_parts();
                self.run_recipe(
                    &fs,
                    Some((&temp_name, temp_kind)),
                    || Self::mark_reconcile_pending(coordinate),
                    |marker| {
                        self.transfer_metadata(temp.inode())?;
                        self.copy_eligible_xattrs(temp.inode(), XattrCopyPolicy::Strict)?;
                        self.transfer_timestamps(temp.inode())?;
                        let workdir_path = self.workdir_root_path()?;
                        self.publish_via_rename(
                            &workdir_path,
                            &temp_name,
                            &upper_dir_path,
                            name,
                            marker,
                            lower.clone(),
                        )
                    },
                )
            }
            InodeType::File => {
                // Full copy-up: private workdir temp, complete
                // metadata/data/xattr transfer, durability, then atomic
                // publication via rename.
                let mode = lower.real_inode().mode()?;
                let temp = fs.create_workdir_temp(
                    name,
                    &upper_dir_path,
                    WorkdirTempRequest::Create {
                        kind: InodeType::File,
                        mode,
                    },
                )?;
                let temp_kind = temp.kind();
                let (temp_name, temp) = temp.into_parts();
                self.run_recipe(
                    &fs,
                    Some((&temp_name, temp_kind)),
                    || Self::mark_reconcile_pending(coordinate),
                    |marker| {
                        self.transfer_metadata(temp.inode())?;
                        self.copy_eligible_xattrs(temp.inode(), XattrCopyPolicy::Strict)?;
                        self.promote_regular_file(temp.inode())?;
                        // Timestamps are replayed after the data stream and
                        // the xattr copy: the upper filesystem refreshed
                        // mtime/ctime on each write/resize, so the replay
                        // restores the lower timestamps before durability and
                        // publication.
                        self.transfer_timestamps(temp.inode())?;
                        // Durability (fsync=auto default; strict/volatile are
                        // future policy choices): the data file is synced
                        // before publication.
                        temp.inode().sync_all()?;
                        // Atomic publication: rename the private workdir temp
                        // onto the upper target name. `Replace` resolves the
                        // stale-upper-entry case; a whiteout at the name is
                        // impossible for authority-only promotion.
                        let workdir_path = self.workdir_root_path()?;
                        self.publish_via_rename(
                            &workdir_path,
                            &temp_name,
                            &upper_dir_path,
                            name,
                            marker,
                            lower.clone(),
                        )
                    },
                )
            }
            InodeType::SymLink => {
                // Symlink promotion side: a workdir symlink temp recreated
                // from the lower target, then xattrs and the atomic rename
                // (only the symlink object itself is copied, never its
                // target).
                let mode = lower.real_inode().mode()?;
                let temp = fs.create_workdir_temp(
                    name,
                    &upper_dir_path,
                    WorkdirTempRequest::Create {
                        kind: InodeType::SymLink,
                        mode,
                    },
                )?;
                let temp_kind = temp.kind();
                let (temp_name, temp) = temp.into_parts();
                self.run_recipe(
                    &fs,
                    Some((&temp_name, temp_kind)),
                    || Self::mark_reconcile_pending(coordinate),
                    |marker| {
                        self.promote_symlink(temp.inode())?;
                        // Symlink metadata transfer: owner/group are
                        // applied; the mode is skipped for symlinks (Linux
                        // `ovl_set_attr` never sets a symlink mode) and the
                        // timestamps are replayed after the xattr copy so no
                        // intermediate step refreshes them.
                        self.transfer_metadata(temp.inode())?;
                        self.copy_eligible_xattrs(temp.inode(), XattrCopyPolicy::Strict)?;
                        self.transfer_timestamps(temp.inode())?;
                        let workdir_path = self.workdir_root_path()?;
                        self.publish_via_rename(
                            &workdir_path,
                            &temp_name,
                            &upper_dir_path,
                            name,
                            marker,
                            lower.clone(),
                        )
                    },
                )
            }
            InodeType::CharDevice
            | InodeType::BlockDevice
            | InodeType::NamedPipe
            | InodeType::Socket => {
                // Special objects are recreated through a workdir `mknod` temp
                // plus metadata/xattrs and the atomic rename. A socket node
                // cannot be recreated through the stable `Inode::mknod`
                // surface (`MknodType` has no socket variant); it is rejected
                // before any side effect (a known VFS-surface limitation, never silently
                // wrong).
                let mknod_type = match lower.real_inode().type_() {
                    InodeType::NamedPipe => MknodType::NamedPipe,
                    InodeType::CharDevice => {
                        let rdev = lower
                            .real_inode()
                            .metadata()?
                            .self_dev_id
                            .ok_or_else(|| {
                                Error::with_message(
                                    Errno::EINVAL,
                                    "the lower char device has no device id",
                                )
                            })?
                            .as_encoded_u64();
                        MknodType::CharDevice(rdev)
                    }
                    InodeType::BlockDevice => {
                        let rdev = lower
                            .real_inode()
                            .metadata()?
                            .self_dev_id
                            .ok_or_else(|| {
                                Error::with_message(
                                    Errno::EINVAL,
                                    "the lower block device has no device id",
                                )
                            })?
                            .as_encoded_u64();
                        MknodType::BlockDevice(rdev)
                    }
                    _ => {
                        return Err(Error::with_message(
                            Errno::EOPNOTSUPP,
                            "socket nodes cannot be copied up",
                        ));
                    }
                };
                let mode = lower.real_inode().mode()?;
                let temp = fs.create_workdir_temp(
                    name,
                    &upper_dir_path,
                    WorkdirTempRequest::Mknod {
                        mode,
                        node: &mknod_type,
                    },
                )?;
                let temp_kind = temp.kind();
                let (temp_name, temp) = temp.into_parts();
                self.run_recipe(
                    &fs,
                    Some((&temp_name, temp_kind)),
                    || Self::mark_reconcile_pending(coordinate),
                    |marker| {
                        self.transfer_metadata(temp.inode())?;
                        self.copy_eligible_xattrs(temp.inode(), XattrCopyPolicy::Strict)?;
                        self.transfer_timestamps(temp.inode())?;
                        // The workdir staging workspace resolves inside the
                        // recipe closure: a resolution failure is a
                        // pre-commit failure, so `run_recipe` best-effort
                        // cleans the staged temp instead of leaking it.
                        let workdir_path = self.workdir_root_path()?;
                        self.publish_via_rename(
                            &workdir_path,
                            &temp_name,
                            &upper_dir_path,
                            name,
                            marker,
                            lower.clone(),
                        )
                    },
                )
            }
            InodeType::Unknown => Err(Error::with_message(
                Errno::EINVAL,
                "cannot promote an overlay object of unknown type",
            )),
        };
        // The no-op `Ok`/`Err` passthrough match is flattened with `?`; on
        // success the transition is committed (any pending reconcile marker
        // is resolved through the passed coordinate — the `CUL` guard is held
        // by the trigger across this call, no re-lock).
        result?;
        coordinate.phase = CopyUpPhase::Idle;
        Ok(())
    }

    /// Publishes the staged workdir temp onto the upper target name via an
    /// atomic rename.
    ///
    /// The shared publication tail of the four `promote` recipe arms
    /// (Dir/File/SymLink/Special): renames the private workdir temp onto the
    /// upper target name with `RenameMode::Replace` (the stale-upper /
    /// `ReconcilePending` residue is atomically replaced instead of failing
    /// `create` with `EEXIST`), commits the physical-upper marker, re-observes
    /// the published upper real object, and runs the semantic authority
    /// publication. The caller holds the `CUL` guard, so the publication
    /// coordinate and the workdir staging workspace are passed in (no `CUL`
    /// re-read); the workdir resolution itself happens inside the recipe
    /// closure, so a resolution failure is classified as a pre-commit
    /// failure and `run_recipe` best-effort cleans the staged temp.
    fn publish_via_rename(
        &self,
        workdir_path: &Path,
        temp_name: &str,
        upper_dir_path: &Path,
        name: &str,
        marker: &mut CommitMarker,
        lower: RealObject,
    ) -> Result<()> {
        workdir_path.rename(temp_name, upper_dir_path, name, RenameMode::Replace)?;
        marker.commit();
        let upper_real = self.upper_real_object(upper_dir_path, name)?;
        self.publish_upper_authority(upper_real, lower)
    }

    /// Runs a fallible upper-mutation recipe with the shared commit scaffold.
    ///
    /// The recipe closure receives the [`CommitMarker`] and calls
    /// [`CommitMarker::commit`] exactly at the physical-upper-commit point
    /// (after the upper rename / exchange / whiteout publish, before the
    /// semantic publication); the commit state is a one-way latch, so the
    /// post-commit-failure-with-`false` state is not representable at the
    /// recipe boundary (a bare boolean can no longer be flipped the wrong
    /// way or read as an arbitrary value). On success the recipe's value is
    /// returned unchanged. On failure the scaffold classifies the outcome:
    /// a failure AFTER the commit runs the `reconcile` closure
    /// (`mark_reconcile_pending` for the copy-up arms,
    /// `invalidate_stale_cache` for the `dir/` recipes); a failure BEFORE the
    /// commit best-effort cleans the staged workdir temp named by `temp`
    /// (`Some((name, kind))`) when one exists — the kind-aware
    /// `cleanup_workdir_temp` dispatches `rmdir` for a directory temp and
    /// `unlink` otherwise, so a directory temp no longer leaks on a
    /// pre-commit failure (its `EISDIR` was previously swallowed); the
    /// cleanup of a never-created name is an ignored `ENOENT`, and `None` —
    /// the plain rename recipe, which stages nothing — is a no-op. The
    /// classification scaffold is used at seven sites (the four `promote`
    /// arms plus `create_over_whiteout`, `remove_target`, and
    /// `rename_upper`), which justifies this single private helper.
    pub(in crate::fs::fs_impls::overlayfs) fn run_recipe<T>(
        &self,
        fs: &Arc<OverlayFs>,
        temp: Option<(&str, InodeType)>,
        reconcile: impl FnOnce(),
        recipe: impl FnOnce(&mut CommitMarker) -> Result<T>,
    ) -> Result<T> {
        let mut marker = CommitMarker::default();
        let recipe_result = recipe(&mut marker);
        match recipe_result {
            Ok(value) => Ok(value),
            Err(err) => {
                if marker.is_committed() {
                    reconcile();
                } else if let Some((temp_name, kind)) = temp {
                    // Pre-commit failure (pre-publication arm): best-effort
                    // kind-aware temp cleanup; residue is a known cleanup
                    // debt, never a visible source.
                    let _ = fs.cleanup_workdir_temp(temp_name, kind);
                }
                Err(err)
            }
        }
    }

    /// Streams the lower regular file's data into the workdir temp.
    ///
    /// The stream runs `read_at`/`write_at` pairs over one reused buffer.
    /// Short reads advance by the read length; a zero-length read before the
    /// declared size or a short write is surfaced as `EIO` — a partial
    /// transfer is never treated as short successful I/O.
    fn promote_regular_file(&self, temp: &Arc<dyn Inode>) -> Result<()> {
        let lower = self.lower_source()?;
        let size = lower.real_inode().size();
        let mut offset = 0usize;
        let mut buffer = vec![0u8; COPY_CHUNK_SIZE];
        while offset < size {
            let chunk = min(COPY_CHUNK_SIZE, size - offset);
            let mut writer = VmWriter::from(&mut buffer[..chunk]).to_fallible();
            let read_len = lower
                .real_inode()
                .read_at(offset, &mut writer, StatusFlags::empty())?;
            if read_len == 0 {
                return_errno_with_message!(
                    Errno::EIO,
                    "the lower source returned a zero-length read before its declared size"
                );
            }
            let mut reader = VmReader::from(&buffer[..read_len]).to_fallible();
            let write_len = temp.write_at(offset, &mut reader, StatusFlags::empty())?;
            if write_len != read_len {
                return_errno_with_message!(
                    Errno::EIO,
                    "the workdir temp accepted a short write during copy-up"
                );
            }
            offset += write_len;
        }
        Ok(())
    }

    /// Recreates the lower symlink target on the workdir temp.
    ///
    /// The lower symlink's target string is read and written onto the temp; a
    /// `SymbolicLink::Path` target (procfs-style) cannot be recreated as a
    /// plain symlink through the stable VFS surface and is rejected with
    /// `EOPNOTSUPP` (never silently wrong).
    fn promote_symlink(&self, temp: &Arc<dyn Inode>) -> Result<()> {
        let lower = self.lower_source()?;
        let target = match lower.real_inode().read_link()? {
            SymbolicLink::Plain(target) => target,
            SymbolicLink::Path(_) => {
                return_errno_with_message!(
                    Errno::EOPNOTSUPP,
                    "a path-style symlink target cannot be copied up"
                );
            }
        };
        temp.write_link(&target)
    }

    /// Transfers the lower metadata onto the upper object: owner, group,
    /// mode, and — for regular files — size.
    ///
    /// The size transfer applies to regular files only: directories report
    /// their own table size and device/socket/FIFO objects have no settable
    /// size. The mode transfer skips symlinks — Linux `ovl_set_attr` never
    /// sets a symlink mode, and the backing filesystems treat a symlink
    /// `set_mode` as a no-op or reject it, so the copy-up skips it rather
    /// than depending on that per-fs behavior. Timestamps are NOT
    /// applied here: the regular-file data stream (`promote_regular_file`)
    /// and the resize refresh mtime/ctime on the upper filesystem, so the
    /// timestamps are replayed by [`OverlayInode::transfer_timestamps`]
    /// after every data/xattr step that could refresh them.
    fn transfer_metadata(&self, temp: &Arc<dyn Inode>) -> Result<()> {
        let lower = self.lower_source()?;
        let lower_inode = lower.real_inode();
        temp.set_owner(lower_inode.owner()?)?;
        temp.set_group(lower_inode.group()?)?;
        if !matches!(lower_inode.type_(), InodeType::SymLink) {
            temp.set_mode(lower_inode.mode()?)?;
        }
        if lower_inode.type_().is_regular_file() {
            temp.resize(lower_inode.size())?;
        }
        Ok(())
    }

    /// Replays the lower timestamps (atime/mtime/ctime) onto the upper
    /// object.
    ///
    /// Split out of [`OverlayInode::transfer_metadata`] so the copy-up
    /// preserves the lower timestamps instead of publishing the copy-up
    /// instant: the File arm replays after the data stream (and the xattr
    /// copy) and before `sync_all`/publication; the data-less arms
    /// (Dir/SymLink/Special) replay after the xattr copy and right before
    /// publication. Every intermediate data/metadata/xattr step that could
    /// refresh mtime/ctime runs before the replay.
    fn transfer_timestamps(&self, temp: &Arc<dyn Inode>) -> Result<()> {
        let lower = self.lower_source()?;
        let lower_inode = lower.real_inode();
        temp.set_atime(lower_inode.atime());
        temp.set_mtime(lower_inode.mtime());
        temp.set_ctime(lower_inode.ctime());
        Ok(())
    }

    /// Copies only `Public` xattrs from the lower source: the
    /// `User`/`Trusted`/`Security` namespaces are enumerated through
    /// `OverlayXattrPolicy::copy_eligible_xattrs` with the caller-selected
    /// [`XattrCopyPolicy`] — the promotion recipe passes the strict
    /// [`XattrCopyPolicy::Strict`]; overlay-private names and the `System`
    /// namespace stay excluded.
    ///
    /// The copy-up policy is strict: a denied source read
    /// (`EACCES`/`EPERM`) propagates and fails the copy-up rather than
    /// silently dropping `security.*`/`trusted.*` metadata. The copy travels
    /// through the mount's creator-credential scope
    /// (`with_creator_credentials_fn`); see
    /// [`OverlayXattrPolicy::copy_eligible_xattrs`](crate::fs::fs_impls::overlayfs::metadata_security::xattr::OverlayXattrPolicy::copy_eligible_xattrs) for the credential
    /// and source-read-policy discussion.
    fn copy_eligible_xattrs(&self, temp: &Arc<dyn Inode>, policy: XattrCopyPolicy) -> Result<()> {
        let lower = self.lower_source()?;
        let fs = self.fs_arc()?;
        fs.policy()
            .credential_policy()
            .with_creator_credentials_fn(|| {
                fs.xattr_policy()
                    .copy_eligible_xattrs(lower.real_inode(), temp, policy)
            })
    }

    /// Publishes the upper authority semantically.
    ///
    /// 1) The `OverlayFs::store_lower_id` step persists the lower-source
    ///    origin record on the upper real inode BEFORE the facts replacement
    ///    (ordering constraint). The step is capability-gated (`Ok(())` with
    ///    no record when gated, never silently wrong) and FALLIBLE — the
    ///    result is propagated unchanged.
    /// 2) The facts are replaced (`upper = upper_real`) under the brief
    ///    `INODE` guard via the [`replace_facts`](OverlayInode::replace_facts)
    ///    transition; the lower-derived `object_id` is kept (constant
    ///    `st_ino`, no re-project-from-upper). The registered inode for the
    ///    current visible-source key is recovered through the
    ///    `OverlayFs::project_new_upper` get-or-create step (one inode per
    ///    key — the recovered inode is this inode, never a duplicate).
    /// 3) The lower page-cache invalidation hook on authority change is a
    ///    future extension point; no field is pre-baked.
    fn publish_upper_authority(
        &self,
        upper_real: RealObject,
        lower_real: RealObject,
    ) -> Result<()> {
        let fs = self.fs_arc()?;
        fs.store_lower_id(upper_real.real_inode(), &lower_real)?;
        let old_facts = self.facts_snapshot();
        // A copied-up DIRECTORY keeps the merged view — the upper directory
        // is created empty (no children are copied), so a copied-up directory
        // that still carries a lower stack must stay `Merged` or the merged
        // readdir and `visible_child_count` would enumerate only the empty
        // upper and the pre-existing lower children would vanish from
        // `getdents` (and the rmdir emptiness gate would pass while lower
        // children still exist). Non-directories keep their pre-copy-up kind
        // (`Single` for every promote path — the condition is keyed on
        // `self.type_().is_directory()`, never on the lower stack alone, so
        // copied-up files/symlinks/specials are not misclassified as
        // `Merged`; the `lowers` are retained regardless so `remove_target`'s
        // pure-upper test and `rename_upper`'s source-fallback compose keep
        // publishing whiteouts).
        let kind = if self.type_().is_directory() && !old_facts.lowers().is_empty() {
            PositiveKind::Merged
        } else {
            old_facts.kind()
        };
        // Keep `upper_real` in scope past the facts construction: it is the
        // post-transition visible source passed to `replace_facts`.
        let new_facts = OverlayObjectFacts::try_new(
            kind,
            Some(upper_real.clone()),
            old_facts.lowers().to_vec(),
        )
        .ok_or_else(|| {
            Error::with_message(Errno::EIO, "cannot construct the post-copy-up facts")
        })?;
        let carrier = fs.project_new_upper(&self.facts_snapshot());
        carrier.replace_facts(new_facts, &upper_real)?;
        Ok(())
    }

    /// Verifies the upper entry at the publication coordinate before reuse
    /// (ReconcilePending path).
    ///
    /// The verification covers the upper entry's object type and basic mode
    /// metadata; the full origin/lower-id verification is the `read_lower_id`
    /// read. A mismatch rejects the reconcile with `EIO`; the caller
    /// surfaces the reconcile/error state. The caller holds the `CUL` guard,
    /// so the publication name is passed in — the helper never re-reads the
    /// coordinate (no non-reentrant lock).
    pub(super) fn verify_upper_target(&self, upper_dir_path: &Path, name: &str) -> Result<()> {
        let upper_real = self.upper_real_object(upper_dir_path, name)?;
        let lower = self.lower_source()?;
        if upper_real.real_inode().type_() != lower.real_inode().type_() {
            return_errno_with_message!(
                Errno::EIO,
                "the upper target type does not match the lower source"
            );
        }
        if upper_real.real_inode().mode()? != lower.real_inode().mode()? {
            return_errno_with_message!(
                Errno::EIO,
                "the upper target mode does not match the lower source"
            );
        }
        Ok(())
    }

    /// Resolves the real object now published at the upper target name
    /// (re-observed through `upper_dir_path.dentry().lookup_child(name)` with
    /// `layer_index` 0 and the upper layer's `fsid`/`container_dev_id`).
    ///
    /// The caller holds the `CUL` guard, so the publication name is passed in
    /// — the helper never re-reads the coordinate (no non-reentrant lock).
    fn upper_real_object(&self, upper_dir_path: &Path, name: &str) -> Result<RealObject> {
        let child_path = Path::new(
            upper_dir_path.mount_node().clone(),
            upper_dir_path
                .dentry()
                .as_dir_dentry_or_err()?
                .lookup_child(name)?,
        );
        let fs = self.fs_arc()?;
        let upper_layer = fs.layer_stack().upper.as_ref().ok_or_else(|| {
            Error::with_message(Errno::EROFS, "the overlay mount has no upper layer")
        })?;
        Ok(RealObject::with_path(
            0,
            RealPath::from_path(&child_path),
            upper_layer.fsid,
            upper_layer.container_dev_id,
        ))
    }

    /// Returns the pinned workdir staging workspace path of this mount.
    ///
    /// Thin delegation to the single `OverlayFs`-level resolver
    /// ([`OverlayFs::workdir_root_path`], `copyup/workdir.rs`) — the claim
    /// resolution (and the EROFS error text) lives in exactly one helper: the
    /// `OverlayInode` entry exists so the copy-up recipe arms resolve the
    /// dentry-anchored workdir staging workspace without re-upgrading the
    /// mount themselves. The workdir path is the rename source of the
    /// `File`/`SymLink`/`Special`/`Dir` publication steps.
    pub(in crate::fs::fs_impls::overlayfs) fn workdir_root_path(&self) -> Result<Path> {
        self.fs_arc()?.workdir_root_path()
    }

    /// Marks the transition [`CopyUpPhase::ReconcilePending`].
    ///
    /// Called on failure after physical publication: the upper object at the
    /// publication coordinate is retained and the next winner entry must
    /// verify it before reuse. The caller holds the `CUL` guard, so the phase
    /// is written through the passed coordinate borrow — no re-lock
    /// (non-reentrant mutex). Invoked by the `File`/`SymLink`/`Special`/`Dir`
    /// recipe arms (four call sites).
    fn mark_reconcile_pending(coordinate: &mut CopyUpTransition) {
        coordinate.phase = CopyUpPhase::ReconcilePending;
    }

    /// Returns the topmost lower real object of this object (`lowers[0]`).
    ///
    /// The `upper.is_some() || !lowers.is_empty()` facts invariant guarantees
    /// the topmost lower exists for a lower-backed object; the checked access
    /// surfaces a structural violation as `EIO` instead of panicking (no
    /// `.unwrap()`/`.expect()` in production paths). The identical selection
    /// runs once in `promote` plus in `promote_regular_file`,
    /// `promote_symlink`, `transfer_metadata`, `copy_eligible_xattrs`, and
    /// `verify_upper_target` (six call sites).
    fn lower_source(&self) -> Result<RealObject> {
        self.facts_snapshot()
            .lowers()
            .first()
            .cloned()
            .ok_or_else(|| {
                Error::with_message(
                    Errno::EIO,
                    "a lower-backed overlay object has no lower source",
                )
            })
    }
}

/// The physical-upper-commit marker of a [`run_recipe`](OverlayInode::run_recipe)
/// recipe closure.
///
/// A small one-way latch over the commit boolean: the recipe calls
/// [`CommitMarker::commit`] exactly once at the physical-upper-commit point
/// (after the upper rename / exchange / whiteout publish, before the
/// semantic publication), and the shared scaffold reads
/// [`CommitMarker::is_committed`] to classify a later failure (reconcile vs
/// pre-publication cleanup). The state transitions `Pending -> Committed` are
/// the only mutations, so the
/// post-commit-failure-with-`false` classification cannot arise by
/// construction — the marker is not a bare boolean at the recipe boundary.
/// Storage: a stack local owned by [`OverlayInode::run_recipe`] and
/// borrowed by the recipe closure for the duration of the recipe call; no
/// lock.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::fs::fs_impls::overlayfs) struct CommitMarker {
    committed: bool,
}

impl CommitMarker {
    /// Marks the physical upper commit (one-way: a committed marker cannot
    /// be un-committed).
    pub(in crate::fs::fs_impls::overlayfs) fn commit(&mut self) {
        self.committed = true;
    }

    /// Returns whether the physical upper commit happened.
    pub(in crate::fs::fs_impls::overlayfs) fn is_committed(&self) -> bool {
        self.committed
    }
}
