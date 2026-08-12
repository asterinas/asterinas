// SPDX-License-Identifier: MPL-2.0

//! The merged-directory readdir index.
//!
//! Single flat module file implementing the readdir surface: the
//! `FileOps::readdir_at` VFS entry for [`OverlayInode`], the
//! index invalidation entry (`invalidate_readdir_index`), the
//! namespace-mutation update entries (`readdir_index_insert` /
//! `readdir_index_remove`), the `visible_child_count` query, the per-directory
//! [`ReaddirIndex`] payload
//! (`entries` / `validity` / `next_cookie` / `tombstone_count`), and the
//! source-observation methods (`readdir_sequence` / `collect_layer_names`).
//!
//! The index is the first source for visible names: exactly one current
//! `ReaddirIndex` per overlay directory (`OverlayInode::readdir_index`,
//! `Some` iff directory); cookies are monotonic and never reused (`1`/`2`
//! reserved for `.`/`..`); the `validity` state machine is two-state only —
//! no `version` field exists.
//!
//! Lock contract: `DIR -> INODE(facts, brief) -> INODE(readdir_index)`; no
//! `INODE` guard is ever held across an underlying call. The synthesized
//! `.`/`..` head entries are served under `DIR` only (they depend on
//! `facts`/`offset`, never on the index), and the `..` parent identity is
//! resolved in that head section so no `INODE` guard is held across the
//! underlying `lookup("..")`; the real-entry visitor walk is invoked under
//! `DIR` + the sleep-capable index lock. This module acquires nothing above
//! `INODE` (no `CUL`, `WL`, `UPPER`, or `MOUNT`).
//!
//! `..` identity note: the `..` entry carries the resolved overlay-parent
//! identity; the projection policy lives in `projection/inode.rs`
//! ([`OverlayInode::resolve_parent_object_id`]) and the head section below
//! only serves its result.

use hashbrown::HashSet;

use super::{
    mount::OverlayFs,
    projection::{OverlayInode, OverlayObjectFacts, PositiveKind, RealObject},
};
use crate::{
    fs::{
        file::InodeType,
        utils::DirentVisitor,
        vfs::{inode::Inode, path::is_dot_or_dotdot},
    },
    prelude::*,
};

/// The Overlay continuation cookie of one visible directory position.
///
/// Monotonic in visible order, strictly increasing, and never reused for a
/// different logical position: `.`/`..` own the reserved cookies `1`/`2`,
/// real entries start at `3`, and `next_cookie` is a never-decreasing
/// high-water mark that survives rebuilds. The cookie is emitted as the
/// user-space `d_off` (procfs-slot convention).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) struct ReaddirCookie(u64);

/// The serve-or-rebuild state of the index (two-state only — no `version`,
/// no third "building" state).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReaddirIndexValidity {
    /// The `Visible` entries are the complete current visible sequence.
    ///
    /// `Valid` guarantees completeness/consistency of the visible sequence only
    /// against overlay-owned mutations; base-fs modifications performed outside
    /// the overlay (silent modifications) are not observed until a rebuild is
    /// triggered, by design (silent base-fs changes are tolerated until the next rebuild).
    Valid,
    /// Serving is refused; the next readdir rebuilds (the conservative
    /// invalidation outcome and the never-publish-partial fallback).
    NeedsRebuild,
}

/// The `INODE`-protected per-directory merged-readdir index payload.
///
/// `entries` are ordered by ascending `cookie` (cookie order == visible
/// order); tombstones keep their old cookie slot, so old-cookie continuation
/// survives deletions. `validity == Valid` implies the `Visible` entries are
/// complete; `next_cookie` is the no-reuse high-water mark; `tombstone_count`
/// drives eager compaction at `>= live_count` (`entries.len() ==
/// tombstone_count + live_count`; array ≤ 2× live — no unbounded tombstones
/// by construction).
///
/// Stored per-directory in `OverlayInode::readdir_index` (present for
/// directories only) and protected by the sleep-capable `INODE`-domain lock.
pub(super) struct ReaddirIndex {
    /// Visible + tombstone slots in ascending cookie order.
    pub(super) entries: Vec<ReaddirIndexEntry>,
    /// Serve-or-rebuild state.
    validity: ReaddirIndexValidity,
    /// Cookie high-water mark; survives rebuilds; never decreases.
    next_cookie: ReaddirCookie,
    /// Number of `Tombstone` slots; eager compaction when `>= live_count`.
    tombstone_count: usize,
}

/// One slot of the index (enum, not a flagged struct).
///
/// The variant types make "a tombstone never holds a strong pin" a
/// compile-time fact (`Tombstone` cannot contain `Arc<OverlayInode>`); a
/// flagged struct would admit a live entry with no inode that only runtime
/// checks could reject. `name`/`cookie` appear in both variants; shared
/// projection is via `match` (one concept, one type — no named intermediate).
pub(super) enum ReaddirIndexEntry {
    /// A live entry: strong pin; `d_off == cookie` (procfs-slot convention);
    /// `d_ino` source; `type_` fixed at scan time and emitted as `d_type`.
    Visible {
        name: String,
        cookie: ReaddirCookie,
        inode: Arc<OverlayInode>,
        type_: InodeType,
    },
    /// A removed entry: reserved cookie + `Weak` inode only — never pins the
    /// removed object; enables same-object revive in place (`insert_visible`)
    /// and old-cookie continuation after deletion.
    Tombstone {
        name: String,
        cookie: ReaddirCookie,
        inode: Weak<OverlayInode>,
    },
}

impl OverlayInode {
    /// The VFS readdir entry.
    ///
    /// Serves the synthesized `.`/`..` head entries (the `..` parent identity
    /// is projected by `projection/inode.rs`) and then the index's visible
    /// real entries in cookie order, skipping `Tombstone` slots. The
    /// serve-or-rebuild contract is [`OverlayInode::ensure_readdir_index`]: a
    /// `Valid` index is served from the cache, a `NeedsRebuild` index is
    /// rebuilt under the same `DIR` transaction. The returned delta advances
    /// the per-FD offset to the next unvisited cookie (`Ok(0)` = end of
    /// sequence). A visitor stop on the FIRST unvisited candidate of a call —
    /// `.` at offset 0 or the first real entry after the head section — is
    /// propagated unchanged: nothing was consumed, the per-FD offset stays
    /// put, and libc either retries or surfaces the error (Linux readdir
    /// semantics; the `legacy_fs.rs` precedent), so an undersized buffer is
    /// never mistaken for end-of-directory. Once this call has consumed at
    /// least one entry, a later visitor stop is not propagated (ext2
    /// precedent) — the consumed delta is returned for continuation
    /// (`..`'s stop after `.` and any real-entry stop after a consumed
    /// head/real entry). Lock-order details are documented inline at each
    /// step.
    pub(in crate::fs::fs_impls::overlayfs) fn readdir_at_impl(
        &self,
        offset: usize,
        visitor: &mut dyn DirentVisitor,
    ) -> Result<usize> {
        // Readdir is supported on overlay directories only (the VFS
        // `InodeHandle`/`getdents` gates this too; the impl keeps the guard).
        let dir = self.dir().ok_or_else(|| Error::new(Errno::ENOTDIR))?;
        // The whole readdir transaction runs under the payload-less `DIR`
        // transaction lock: observation, build, and publication are
        // serialized per directory (one-`DIR` rule).
        let _dir_guard = dir.lock();
        // Brief `INODE` facts snapshot (`DIR -> INODE`; the guard is released
        // before any lock-free use).
        let facts = self.facts_snapshot();
        // `usize -> u64` is always lossless.
        let input_cookie = ReaddirCookie(offset as u64);
        // Serve-or-rebuild under the same `DIR` transaction.
        // `ensure_readdir_index` returns the facts the index was actually
        // published from — the passed-in snapshot on a plain serve, or the
        // revalidated snapshot on the race path — so the `..` head
        // below resolves the parent identity from the same facts as the
        // index (a single readdir never serves a `d_ino("..")` contradicting
        // its own index).
        let facts = self.ensure_readdir_index(&facts)?;
        let mut last_visited: Option<ReaddirCookie> = None;
        // The returned delta is `last_visited_cookie - input`; a visited
        // cookie is always `> input`, so the subtraction cannot underflow.
        // `u64 -> usize` uses `try_from` (all supported arches are 64-bit —
        // an explicit platform assumption).
        let delta_fn = |last_visited: Option<ReaddirCookie>| -> usize {
            let delta = match last_visited {
                Some(last) => last.0 - input_cookie.0,
                None => 0,
            };
            usize::try_from(delta).unwrap_or(usize::MAX)
        };
        // Reserved head cookies: `.` (1) and `..` (2) are synthesized
        // special entries and never appear in the `entries`
        // array. They depend only on `facts` and `offset` — never on the
        // index — so they are served BEFORE the index `INODE` lock is taken
        // (the real-entry walk below takes it). The `..` parent identity is
        // resolved only in the branch that actually serves `..`, i.e. after
        // the `.` visit above accepted (at offset 0, a visitor stop at `.`
        // never triggers the resolution), and still before any `INODE`
        // guard — no guard is held across the underlying `lookup("..")`
        // (the approximation `d_ino("..") == d_ino(".")` is the route's
        // fallback). This keeps every offset-0/1 readdir free of live
        // `lookup("..")`/xattr reads when `..` is never served, and removes
        // the spurious warning path.
        if input_cookie < ReaddirCookie(1) {
            // `.` carries this directory's projected identity. A failure here
            // is propagated unchanged: this call has consumed nothing, so the
            // per-FD offset stays put and libc either retries or surfaces the
            // error — an undersized buffer is never mistaken for
            // end-of-directory (Linux readdir and the `legacy_fs.rs`
            // precedent).
            visitor.visit(".", self.ino(), InodeType::Dir, 1)?;
            last_visited = Some(ReaddirCookie(1));
        }
        if input_cookie < ReaddirCookie(2) {
            // `..` carries the overlay-parent identity, resolved here — in
            // the branch that emits `..` — before the index lock (no `INODE`
            // guard across the underlying `lookup("..")`).
            let parent_object_id = self.resolve_parent_object_id(&facts);
            if visitor
                .visit("..", parent_object_id.ino, InodeType::Dir, 2)
                .is_err()
            {
                // `.` was already consumed by this call, so the consumed
                // delta is returned and the caller continues from the next
                // offset (the resume semantics are preserved here).
                return Ok(delta_fn(last_visited));
            }
            last_visited = Some(ReaddirCookie(2));
        }
        // Real entries (cookies >= 3): served under the sleep-capable index
        // `INODE` lock; the first cookie above the input, then walk in cookie
        // order skipping `Tombstone` slots and visiting `Visible` entries
        // only.
        let index = self
            .readdir_index()
            .ok_or_else(|| Error::new(Errno::ENOTDIR))?;
        let index = index.lock();
        let start = index
            .first_entry_after(input_cookie)
            .unwrap_or(index.entries.len());
        for entry in &index.entries[start..] {
            let ReaddirIndexEntry::Visible {
                name,
                cookie,
                inode,
                type_,
            } = entry
            else {
                // A tombstone is a traversal-skipped placeholder, never a
                // visible entry.
                continue;
            };
            let d_off = match usize::try_from(cookie.0) {
                Ok(d_off) => d_off,
                // Unreachable on supported 64-bit targets (explicit platform
                // assumption); a cookie beyond `usize` cannot be served.
                Err(_) => break,
            };
            // `d_ino` derives from the shared identity policy: the child
            // inode's precomputed `object_id` (`Inode::ino()`).
            if let Err(err) = visitor.visit(name, inode.ino(), *type_, d_off) {
                if last_visited.is_none() {
                    // This call has consumed nothing yet: propagate the
                    // visitor error so a full-but-undersized buffer is not
                    // mistaken for end-of-directory (the per-FD offset
                    // stays put and libc retries or surfaces the error).
                    return Err(err);
                }
                // The visitor stopped after this call already consumed
                // entries; the error is not propagated (ext2 precedent); the
                // consumed delta is returned.
                break;
            }
            last_visited = Some(*cookie);
        }
        // The delta lands the per-FD offset exactly on the next unvisited
        // cookie (`Ok(0)` = end of sequence).
        Ok(delta_fn(last_visited))
    }
}

impl OverlayInode {
    /// Invalidates the index: marks it `NeedsRebuild`.
    ///
    /// Called by the namespace-mutation module — and by copy-up after a
    /// directory authority transition that changes the visible source set —
    /// under this directory's `DIR`, after the underlying namespace commit
    /// and before `DIR` release. Takes only the index `INODE` lock; no
    /// `version` exists to bump.
    pub(super) fn invalidate_readdir_index(&self) {
        if let Some(index) = self.readdir_index() {
            index.lock().validity = ReaddirIndexValidity::NeedsRebuild;
        }
    }

    /// Inserts a freshly visible name (create/mkdir/mknod/symlink/link
    /// publication) into a `Valid` index without a full rebuild.
    ///
    /// Decision rule: the fine-grained insert runs only when the parent index
    /// is `Valid` AND the parent is upper-only (`facts.kind == Single`,
    /// `upper.is_some()`, `lowers.is_empty()`); every other case marks
    /// `NeedsRebuild` (conservative floor — a merged/lower-backed parent or a
    /// stale index cannot provably keep the cookie order). The
    /// `insert_visible` verdict is consumed — a same-object revive keeps the
    /// index `Valid`; a fresh CREATE append (whose end-of-order position
    /// cannot be proven here) falls back to `NeedsRebuild`. The caller holds
    /// this directory's `DIR`; the method snapshots the facts briefly (INODE,
    /// released) and then takes the index `INODE` lock (intra-INODE order:
    /// facts before index, never simultaneous).
    pub(super) fn readdir_index_insert(
        &self,
        name: &str,
        inode: Arc<OverlayInode>,
        type_: InodeType,
    ) {
        let facts = self.facts_snapshot();
        let Some(index) = self.readdir_index() else {
            return;
        };
        let mut index = index.lock();
        if index.validity == ReaddirIndexValidity::Valid
            && facts.kind() == PositiveKind::Single
            && facts.upper().is_some()
            && facts.lowers().is_empty()
        {
            // The revive-vs-create result is consumed at this call boundary. A
            // same-object revive keeps the index `Valid` (same cookie slot,
            // provably same position); a fresh CREATE append cannot be proven
            // end-of-order here (this helper has no position evidence), so it
            // falls back to the conservative `NeedsRebuild` floor instead of
            // trusting the append position.
            if !index.insert_visible(name, inode, type_) {
                index.validity = ReaddirIndexValidity::NeedsRebuild;
            }
        } else {
            index.validity = ReaddirIndexValidity::NeedsRebuild;
        }
    }

    /// Removes a hidden/removed name (unlink/rmdir publication) from a
    /// `Valid` index without a full rebuild.
    ///
    /// Decision rule: when the index is `Valid` the tombstone rule applies —
    /// the entry is converted to a `Tombstone` in place, keeping its cookie
    /// slot reserved; a `NeedsRebuild` index stays as-is (the next readdir
    /// rebuilds wholesale). The `remove_visible` verdict is consumed — a
    /// `Valid` index that cannot tombstone the removed name violates its
    /// completeness claim, so the conservative `NeedsRebuild` floor applies.
    pub(super) fn readdir_index_remove(&self, name: &str) {
        let Some(index) = self.readdir_index() else {
            return;
        };
        let mut index = index.lock();
        if index.validity == ReaddirIndexValidity::Valid {
            // The removal result is consumed — a `Valid` index that cannot
            // tombstone the removed name violates its completeness claim, so
            // the conservative rebuild floor applies.
            if !index.remove_visible(name) {
                index.validity = ReaddirIndexValidity::NeedsRebuild;
            }
        }
    }

    /// Counts the visible children (the rmdir emptiness gate).
    ///
    /// Ensures the index is `Valid` (rebuild under the same `DIR` transaction
    /// — the caller holds it) and counts the `Visible` entries (the real
    /// children; `.`/`..` are synthesized heads and never appear in the
    /// `entries` array).
    pub(super) fn visible_child_count(&self, facts: &OverlayObjectFacts) -> Result<usize> {
        self.ensure_readdir_index(facts)?;
        let index = self.readdir_index().ok_or_else(|| {
            Error::with_message(Errno::ENOTDIR, "the overlay inode is not a directory")
        })?;
        let index = index.lock();
        Ok(index
            .entries
            .iter()
            .filter(|entry| matches!(entry, ReaddirIndexEntry::Visible { .. }))
            .count())
    }

    /// Returns the per-directory index lock, if this object is a directory.
    pub(super) fn readdir_index(&self) -> Option<&Mutex<ReaddirIndex>> {
        self.readdir_index.as_ref()
    }

    /// Ensures the directory's index is `Valid` under the caller's `DIR`
    /// transaction (the build leg).
    ///
    /// Returns the facts the index was published from — the passed-in
    /// snapshot on a plain serve of a `Valid` index, or the revalidated
    /// snapshot the rebuild scanned — so the caller serves `.`/`..` from the
    /// same facts as the index. A `NeedsRebuild` index is rebuilt
    /// out-of-lock (the source observation may sleep) with a race-revalidation
    /// retry; a persistent mismatch surfaces `EIO` and never publishes a stale
    /// index, and a failed scan leaves the previous `Valid` index intact
    /// (lock-order details are documented inline).
    pub(super) fn ensure_readdir_index(
        &self,
        facts: &OverlayObjectFacts,
    ) -> Result<OverlayObjectFacts> {
        let index = self.readdir_index().ok_or_else(|| {
            Error::with_message(Errno::ENOTDIR, "the overlay inode is not a directory")
        })?;
        {
            let index = index.lock();
            if index.validity == ReaddirIndexValidity::Valid {
                return Ok(facts.clone());
            }
        }
        // Rebuild out-of-lock: no `INODE` guard is held across the
        // underlying reads.
        let mut scan_facts = facts.clone();
        let mut retried = false;
        let sequence = loop {
            let sequence = self.readdir_sequence(&scan_facts)?;
            // Revalidate the facts snapshot after the released-lock segment:
            // the snapshot is compared on its observable layer composition
            // (kind + per-layer fsid/real-ino), the only equality signals
            // available at the overlayfs ceiling.
            let revalidated = self.facts_snapshot();
            let unchanged = {
                let same_kind = scan_facts.kind() == revalidated.kind();
                let same_upper = match (scan_facts.upper(), revalidated.upper()) {
                    (Some(left), Some(right)) => {
                        left.fsid() == right.fsid()
                            && left.real_inode().ino() == right.real_inode().ino()
                    }
                    (None, None) => true,
                    _ => false,
                };
                let same_lowers =
                    scan_facts.lowers().len() == revalidated.lowers().len()
                        && scan_facts.lowers().iter().zip(revalidated.lowers()).all(
                            |(left, right)| {
                                left.fsid() == right.fsid()
                                    && left.real_inode().ino() == right.real_inode().ino()
                            },
                        );
                same_kind && same_upper && same_lowers
            };
            if unchanged {
                break sequence;
            }
            if retried {
                // Persistent mismatch: a copy-up transition kept racing the
                // rebuild; never publish a stale index — leave
                // `NeedsRebuild` and surface the refusal (`EIO` is the tree's
                // consistency-refusal convention).
                return Err(Error::with_message(
                    Errno::EIO,
                    "the overlay directory facts changed while the readdir index was being rebuilt",
                ));
            }
            retried = true;
            scan_facts = revalidated;
        };
        // Publish the complete sequence under the index `INODE` lock:
        // `rebuild` reads old cookies from the surviving entries and keeps
        // or allocates them per the keep-rule.
        index.lock().rebuild(sequence);
        // Return the facts the index was published from: the passed-in
        // snapshot on a plain serve, or the (possibly revalidated) snapshot
        // the rebuild scanned and verified against — so the caller resolves
        // the `..` identity from the same facts as the index.
        Ok(scan_facts)
    }

    /// Observes the current visible sequence of this directory from the
    /// pinned layer real objects (the build leg).
    ///
    /// Merged (`facts.kind() == Merged`): enumerate the upper (when present),
    /// then each lower top→bottom; the downward merge stops after the first
    /// layer whose real directory is opaque (`is_opaque_directory()` — the
    /// opaque layer's own names are still emitted and a deeper lower's names
    /// never leak through the barrier). Single
    /// (`facts.kind() == Single`): enumerate the single visible source (the
    /// upper, or `lowers[0]`). Per new name:
    /// `OverlayFs::lookup_binding` resolves the merged visibility — a
    /// positive binding contributes the shared [`OverlayInode`] (and its
    /// scan-time `type_`); a negative binding (whiteout/opaque evidence) is
    /// recorded in the local `seen` set and skipped. Dedup key is the visible
    /// name. `.`/`..` are never scanned. The `seen: HashSet<String>` and the
    /// returned `Vec<(String, Arc<OverlayInode>, InodeType)>` are pure locals
    /// / an anonymous tuple reuse of final payload types. Runs out-of-lock
    /// inside the caller's `DIR` transaction; the caller re-validates the
    /// facts snapshot before publication.
    fn readdir_sequence(
        &self,
        facts: &OverlayObjectFacts,
    ) -> Result<Vec<(String, Arc<OverlayInode>, InodeType)>> {
        // Bind the mount Arc before downcasting: the `Inode::fs()` upgrade
        // returns an owned `Arc<dyn FileSystem>` whose temporary would drop
        // before the borrowed `&OverlayFs` could be used (a live OverlayInode
        // keeps its mount alive).
        let fs = self.fs();
        let fs = fs.downcast_ref::<OverlayFs>().ok_or_else(|| {
            Error::with_message(
                Errno::EIO,
                "the overlay inode is not backed by an overlay mount",
            )
        })?;
        // The layers to enumerate in scan order. The layer list is
        // a local (not a named type): `Single` -> the single visible source;
        // `Merged` -> the upper (when present) then the lowers top→bottom,
        // stopping after the first opaque layer.
        let layers: Vec<&RealObject> = match facts.kind() {
            PositiveKind::Single => {
                let source = match facts.upper() {
                    Some(upper) => upper,
                    // The `upper.is_some() || !lowers.is_empty()` facts
                    // invariant guarantees `lowers[0]`.
                    None => &facts.lowers()[0],
                };
                vec![source]
            }
            PositiveKind::Merged => {
                // Enumerate the upper (when present), then the lowers
                // top→bottom. The downward merge stops after the first layer
                // whose real directory is opaque (`is_opaque_directory()`),
                // matching Linux `ovl_iterate` / `ovl_dir_read_merged`: a
                // deeper lower's names must never leak through an opaque
                // barrier. The opaque layer's own names are still emitted.
                let mut layers = Vec::new();
                for layer in facts.upper().into_iter().chain(facts.lowers().iter()) {
                    layers.push(layer);
                    if layer.is_opaque_directory()? {
                        break;
                    }
                }
                layers
            }
        };
        let mut seen = HashSet::new();
        let mut sequence = Vec::new();
        for layer in layers {
            for name in self.collect_layer_names(layer)? {
                // Dedup by visible name: a name observed in an upper
                // layer is never re-emitted from a lower layer, and a
                // whiteout/opaque-hidden name is recorded and skipped.
                if !seen.insert(name.clone()) {
                    continue;
                }
                // Per-name visibility resolution: a positive binding
                // contributes the shared Overlay inode; a negative binding
                // (whiteout/opaque evidence) hides the name.
                if let Some(inode) = fs.lookup_binding(facts, &name)?.binding.into_inode() {
                    let file_type = inode.type_();
                    sequence.push((name, inode, file_type));
                }
            }
        }
        Ok(sequence)
    }

    /// Delegates the underlying readdir of one pinned real layer directory.
    ///
    /// Collects the non-dot names in underlying enumeration order; underlying
    /// errors propagate. `.`/`..` are never scanned. No Overlay lock is held
    /// across the underlying call; the caller's `DIR` transaction covers it.
    fn collect_layer_names(&self, layer: &RealObject) -> Result<Vec<String>> {
        let mut names: Vec<String> = Vec::new();
        layer.real_inode().readdir_at(0, &mut names)?;
        names.retain(|name| !is_dot_or_dotdot(name));
        Ok(names)
    }
}

impl ReaddirIndex {
    /// Constructs the empty initial index.
    ///
    /// `entries` empty, `validity == NeedsRebuild`, `next_cookie ==
    /// ReaddirCookie(3)` (cookies `1`/`2` are reserved for `.`/`..`),
    /// `tombstone_count == 0`. The `OverlayInode::readdir_index` field
    /// initializes every directory with this constructor.
    pub(super) fn new() -> Self {
        Self {
            entries: Vec::new(),
            validity: ReaddirIndexValidity::NeedsRebuild,
            next_cookie: ReaddirCookie(3),
            tombstone_count: 0,
        }
    }

    /// Rebuilds the index from a complete visible sequence.
    ///
    /// Cookie assignment: keep a name's previous cookie iff the name was
    /// `Visible` before AND the old entry's inode is the same logical object
    /// as the new binding (`Arc::ptr_eq`) AND the previous cookie is above
    /// the last assigned cookie (greedy order-preserving); every other
    /// appearance allocates a fresh cookie from the never-decreasing
    /// `next_cookie`. Drops ALL tombstones and resets `tombstone_count`; sets
    /// `validity = Valid`. Never renumbers already-exposed cookies. Old
    /// cookies are read from the surviving entries while the scan runs — no
    /// separate capture map.
    fn rebuild(&mut self, sequence: Vec<(String, Arc<OverlayInode>, InodeType)>) {
        let mut entries = Vec::with_capacity(sequence.len());
        // `.`/`..` own cookies 1/2; a kept cookie must stay above the last
        // assigned cookie so the array remains in ascending cookie order.
        let mut last_assigned = ReaddirCookie(2);
        for (name, inode, type_) in sequence {
            let previous = self.entries.iter().find_map(|old| match old {
                ReaddirIndexEntry::Visible {
                    name: old_name,
                    cookie: old_cookie,
                    inode: old_inode,
                    ..
                } if old_name == &name && Arc::ptr_eq(old_inode, &inode) => Some(*old_cookie),
                _ => None,
            });
            let cookie = match previous {
                Some(previous) if previous > last_assigned => previous,
                _ => {
                    let fresh = self.next_cookie;
                    // Checked/saturating arithmetic: the cookie space cannot
                    // be exhausted by any real directory; the high-water mark
                    // saturates instead of wrapping.
                    self.next_cookie = ReaddirCookie(self.next_cookie.0.saturating_add(1));
                    fresh
                }
            };
            last_assigned = cookie;
            entries.push(ReaddirIndexEntry::Visible {
                name,
                cookie,
                inode,
                type_,
            });
        }
        self.entries = entries;
        self.validity = ReaddirIndexValidity::Valid;
        self.tombstone_count = 0;
    }

    /// Returns the index of the first entry whose cookie is above `cookie`
    /// (`partition_point` on the cookie-ordered array).
    fn first_entry_after(&self, cookie: ReaddirCookie) -> Option<usize> {
        let index = self.entries.partition_point(|entry| match entry {
            ReaddirIndexEntry::Visible {
                cookie: entry_cookie,
                ..
            }
            | ReaddirIndexEntry::Tombstone {
                cookie: entry_cookie,
                ..
            } => *entry_cookie <= cookie,
        });
        (index < self.entries.len()).then_some(index)
    }

    /// Converts the `Visible` entry `name` into a `Tombstone` in place.
    ///
    /// O(n) by-name find (the dominant maintenance cost; an auxiliary
    /// name→index map is a future optimization, NOT implemented — no
    /// premature optimization). The strong pin becomes a `Weak` that never
    /// keeps the removed object alive; the cookie slot stays reserved;
    /// `tombstone_count` increments; `validity` stays `Valid`. Eager
    /// compaction runs when `tombstone_count >= live_count` (amortized O(1)
    /// per removal, memory ≤ 2× live). Returns whether an entry was removed.
    ///
    /// `#[must_use]`: the returned verdict is consumed by every current
    /// caller (`readdir_index_remove` falls back to `NeedsRebuild` on
    /// `false`) and must not be silently discarded by future callers.
    #[must_use]
    pub(super) fn remove_visible(&mut self, name: &str) -> bool {
        let Some(index) = self.entries.iter().position(|entry| {
            matches!(
                entry,
                ReaddirIndexEntry::Visible { name: entry_name, .. } if entry_name == name
            )
        }) else {
            return false;
        };
        let (name, cookie, inode) = match &self.entries[index] {
            ReaddirIndexEntry::Visible {
                name,
                cookie,
                inode,
                ..
            } => (name.clone(), *cookie, inode.clone()),
            _ => return false,
        };
        self.entries[index] = ReaddirIndexEntry::Tombstone {
            name,
            cookie,
            inode: Arc::downgrade(&inode),
        };
        self.tombstone_count += 1;
        // `entries.len() == tombstone_count + live_count`.
        if self.tombstone_count >= self.entries.len() - self.tombstone_count {
            self.compact_tombstones();
        }
        true
    }

    /// Strict revive-vs-create.
    ///
    /// REVIVE (returns `true`): a `Tombstone` with `name` exists AND its
    /// `Weak::upgrade()` succeeds AND `Arc::ptr_eq(upgraded, &inode)` (the
    /// same logical object) — keep the cookie, convert to `Visible`, restore
    /// the strong pin and `type_`, decrement `tombstone_count`. CREATE
    /// (returns `false`): append a new `Visible` entry at the end with a
    /// fresh cookie from `next_cookie`. The caller must only use the create
    /// path when it can prove the new name's correct visible position is the
    /// end of the cookie order; a mid-sequence insert must instead mark
    /// `NeedsRebuild` — never renumber already-exposed cookies.
    ///
    /// `#[must_use]`: the revive-vs-create verdict is consumed by every
    /// current caller (`readdir_index_insert` falls back to `NeedsRebuild` on
    /// the create path) and must not be silently discarded by future callers.
    #[must_use]
    pub(super) fn insert_visible(
        &mut self,
        name: &str,
        inode: Arc<OverlayInode>,
        type_: InodeType,
    ) -> bool {
        if let Some(index) = self.entries.iter().position(|entry| {
            matches!(
                entry,
                ReaddirIndexEntry::Tombstone { name: entry_name, .. } if entry_name == name
            )
        }) {
            // Clone the revive data first: the tombstone borrow must end
            // before `self.entries[index]` is mutated in place.
            let revive = match &self.entries[index] {
                // The pattern binding is renamed (`weak_inode`) so the
                // `Arc::ptr_eq` below compares the upgraded tombstone against
                // the passed `inode` parameter, not against the `Weak`.
                ReaddirIndexEntry::Tombstone {
                    name,
                    cookie,
                    inode: weak_inode,
                } => match weak_inode.upgrade() {
                    Some(upgraded) if Arc::ptr_eq(&upgraded, &inode) => {
                        Some((name.clone(), *cookie))
                    }
                    _ => None,
                },
                _ => None,
            };
            if let Some((name, cookie)) = revive {
                self.entries[index] = ReaddirIndexEntry::Visible {
                    name,
                    cookie,
                    inode,
                    type_,
                };
                self.tombstone_count -= 1;
                return true;
            }
            // A same-name tombstone that no longer resolves to the same
            // object cannot be revived in place: fall through to the CREATE
            // path with a fresh cookie.
        }
        // CREATE: append at the end with a fresh cookie from the high-water
        // mark (the caller proves the end-of-order position).
        let cookie = self.next_cookie;
        self.next_cookie = ReaddirCookie(self.next_cookie.0.saturating_add(1));
        self.entries.push(ReaddirIndexEntry::Visible {
            name: name.into(),
            cookie,
            inode,
            type_,
        });
        false
    }

    /// Drops all tombstones in place.
    ///
    /// Private, invariant-preserving: O(n) in-place filter under the
    /// already-held index `INODE` lock inside the same `DIR` transaction —
    /// keeps the `Visible` entries (order and cookies untouched), drops ALL
    /// tombstones, resets `tombstone_count`; `validity` stays `Valid`. No
    /// BIO, no source scan: adds no new lock hazard.
    fn compact_tombstones(&mut self) {
        self.entries
            .retain(|entry| matches!(entry, ReaddirIndexEntry::Visible { .. }));
        self.tombstone_count = 0;
    }
}
