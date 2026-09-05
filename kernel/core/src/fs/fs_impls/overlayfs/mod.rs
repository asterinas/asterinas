// SPDX-License-Identifier: MPL-2.0

//! Overlayfs is a union filesystem: each mount merges one writable upper
//! directory with one or more read-only lower directories into a single
//! directory tree.
//!
//! Every mutation is served by the upper — an object still provided by a
//! lower layer is copied up first, and lower layers are never modified. The
//! vocabulary table below is the definition home for the terms every module
//! shares.
//!
//! # Layer model and mount flow
//!
//! A mount stacks real directories into a fixed merge order: at most one
//! writable **upper** on top, the read-only **lowers** beneath it
//! (topmost-first), and — on writable mounts — a **workdir** on the upper's
//! filesystem. The workdir hosts the private staging area used by copy-up
//! (the lower-to-upper promotion of a written object) and by name removals.
//!
//! Mount-time build order — everything runs once, in `OverlayFs::new`:
//!
//! 1. Parse the mount options into validated intent; unknown keys and
//!    conflicting combinations fail here, and an explicitly requested
//!    unimplemented feature is accepted only with a one-shot warning that
//!    the mount proceeds without it.
//! 2. Resolve every layer root, including the workdir, and reject
//!    non-directories.
//! 3. Validate the original resolved paths: workdir-vs-upperdir mount and
//!    underlying filesystem, root overlap, and workdir-vs-lowers overlap.
//! 4. Build the layer stack from the roots' dentries and backing filesystems.
//! 5. Exclusively claim the upper and workdir roots so two overlays cannot
//!    share them.
//! 6. Prepare the staging workspace on writable mounts.
//! 7. Probe what the upper filesystem can store, validate the required
//!    capabilities, and persist the overlay uuid record when effective
//!    (writable mounts only).
//! 8. Publish the mount policy: the fixed per-mount decisions that every
//!    later operation reads.
//!
//! # Vocabulary
//!
//! | Term | Meaning | Code carrier |
//! |---|---|---|
//! | upper / lower | The writable top directory and the read-only directories beneath it; the merge order is fixed at mount time, upper first, lowers topmost-first. | `LayerStack::upper`, `LayerStack::lowers` (`layer.rs`) |
//! | layer | One pinned real directory root of the mount, held as its root dentry together with the filesystem it is rooted on. | `Layer` (`layer.rs`) |
//! | layer stack | The ordered, immutable collection of layers assembled once at mount time. | `LayerStack` (`layer.rs`) |
//! | fsid | The per-mount ordinal of one underlying filesystem, shared by every layer rooted on it; the layer table keeps it beside the layer's device and pinned root inode. | `IdentityPolicy`'s layer table (`LayerNumbers`, `inode/identity.rs`) |
//! | real object | One underlying filesystem entry as seen from a known layer; the side it sits on is a value, not a type: an upper object is an entry of the upper layer's tree and carries `UPPER_LAYER_INDEX` (0), while a lower object is an entry of one lower layer and carries that layer's index. Its two exits — the real inode and the real dentry — are not side-restricted by the type: that a write issued from them must come from an already-promoted take point is a discipline of the write paths (see `real.rs`), not a type-level restriction. | `RealObject` (`real.rs`) |
//! | real-object stack | The complete real-object composition behind one logical object: an optional upper object plus the retained lower objects. | `RealObjectStack` (`layer.rs`) |
//! | visible source | The topmost real object of a stack — the upper when present, else the topmost lower; it provides the visible metadata and the directory bit, while the identity key comes from the stable identity. One `RealObject` is shared by all three read-side consumers: the read take point, this visible source, and the readdir merge chain. | `RealObjectStack::visible_source` (`layer.rs`, returns `&RealObject`), `OverlayInode::real_object` (`inode/mod.rs`) |
//! | merged directory | A directory whose visible names unite upper and lower contributions. | `OverlayInode::build_merged_entries` (`inode/readdir.rs`) |
//! | logical object | The overlay inode published to the VFS, shared by every name bound to the same real-object stack. | `OverlayInode` (`inode/mod.rs`) |
//! | projection | Creating or reusing the shared logical inode for one real-object stack: it resolves the stable identity, draws the published number once, and moves the retained real objects in. | `OverlayFs::project_inode` (`inode/lookup.rs`) |
//! | published identity | The precomputed `st_dev`/`st_ino` pair a logical object reports, kept stable across copy-up because it is derived from the object's stable identity rather than from whichever side currently shows. It is drawn once, when the instance is projected, and a copy-up that persists no origin record moves that instance's cache entry onto the key its identity now resolves to instead of drawing the pair again. | `ObjectVisibleId` (`inode/identity.rs`), `InodeCache::rekey` (`inode/inode_cache.rs`) |
//! | xino | The encoding that packs a layer's fsid into the high bits of the published inode number; its effective form is resolved once at mount into one of three: every layer on one filesystem publishes the layer's own numbers, the encoding carries the fsid over the overlay's own device, or a directory takes a number of the mount's own range and anything else keeps its layer's. | `IdentityPolicy::new`, `IdentityPolicy::project` (`inode/identity.rs`) |
//! | real id | The pair (layer `fsid` plus real inode number) that every name bound to one real object resolves to: its durable origin record when it has one, else its retained topmost lower, else its own real object; it survives copy-up and is the key the identity-reuse cache files the object under. | `ObjectRealId`, `IdentityPolicy::real_id_of` (`inode/identity.rs`), `InodeCache` (`inode/inode_cache.rs`) |
//! | origin-preserved | The identity-provenance bit: this object's stable identity comes from the lower side rather than from its own upper. One lenient entry answers it — `OverlayFs::origin_of` folds an absent, undecodable, or unreadable record into the pure-upper answer, warning once on the unreadable one — and its four callers are the projection's identity step, the merged-snapshot read, and the two impurity gates of the link and rename recipes. | `OverlayFs::origin_of` (`inode/identity.rs`), `resolve_entry_origins` (`inode/readdir.rs`), `inode/lookup.rs`, `inode/dir/{mod,rename}.rs` |
//! | layer-internal probe | Probing the receiver's own lower layers by name for whether that name physically exists there: a whiteout is a miss, an unanswerable error counts as a hit (so a removal driven by it never leaves a name behind), and it answers layer-internal existence only — a different question from `origin-preserved`. | `OverlayInode::has_lower_entry` (`inode/lookup.rs`) |
//! | claim | The mount-time exclusive lease on the upper and workdir root inodes; a second overlay claiming either is refused. | `UpperWorkdirInuse` (`fs/mount/inuse.rs`) |
//! | workdir | The claimed directory on the upper's filesystem; its prepared `<workdir>/work` staging workspace hosts all private staging objects. | `UpperWorkdirInuse::prepare_workdir` (`fs/mount/inuse.rs`) |
//! | workdir temp | A staged private object created in the staging workspace, later published by rename or cleaned up by kind. | `WorkdirTemp`, `UpperWorkdirInuse::create_workdir_temp` (`inode/copyup/workdir.rs`) |
//! | copy-up | The lower-to-upper promotion of a written object: collect the lower-only ancestors from the operation's overlay dentry, then promote them top down, each round staging a private workdir temp and atomically renaming it into place under its own publication parent, after re-reading that object's and its publication parent's name-taken latches under both transaction locks. | `OverlayFs::copy_up_at` (`inode/copyup/mod.rs`) |
//! | writable take point | The one way a write is admitted: the read-only gate (`EROFS`), the copy-up promotion sourced from the caller's own dentry, and the upper real object the operation is handed. The overlay evaluates no DAC of its own — the VFS owns that — so every write path converges on this single take point. | `OverlayInode::writable_real_object`, `OverlayInode::writable_upper` (`inode/mod.rs`) |
//! | per-inode transaction lock | The per-inode mutex that serializes one object's mutations; a directory's payload is its current readdir snapshot slot, holding `None` while no snapshot is built or after one is invalidated, and it also serializes an object's promotion against the removal or displacement of its name. The payload's spelling goes through the `OverlayInodeLockPayload` alias, whose name deliberately does not say what the lock carries: a site that takes this lock only to serialize must not read the payload because of it. | `OverlayInode.lock`, `OverlayInode::lock` (`inode/mod.rs`, `inode/dir/mod.rs`) |
//! | name-taken latch | The one-way per-object fact that this object's name binding was taken away from its parent, or that the object was displaced at that name: set by the removal and overwrite paths under the object's own transaction lock, never cleared, and read by each copy-up round under it. | `OverlayInode.name_taken` (`inode/mod.rs`) |
//! | recipe | One namespace-mutation procedure (create, link, remove, rename) that runs under the parent's transaction lock, on objects the entry has already taken through the writable take point. | `inode/dir/{create,link,remove,rename}.rs` |
//! | whiteout | A name-level visibility barrier published in the upper to hide a lower-backed name; either a char device `0:0` or a marked regular file. | `OverlayFs::publish_whiteout` (`inode/dir/whiteout.rs`), `is_whiteout_inode` (`inode/lookup.rs`) |
//! | whiteout cache | The mount-time shared whiteout handle plus the one-way link-capability latch; a publish links from the shared handle or renames a one-shot workdir temp. | `WhiteoutCache` (`inode/dir/whiteout.rs`) |
//! | opaque directory | A directory-level barrier: a real directory whose private record cuts off every lower contribution beneath it. | `is_opaque_directory` (`inode/lookup.rs`) |
//! | stale upper | An upper-backed record whose fresh layer truth no longer resolves is surfaced as `ESTALE` by translating the real-layer `ENOENT`; there is no proactive stale detection and no cache rebuild. | `translate_stale_upper_enoent` (`inode/dir/remove.rs`) |
//! | private record | An overlay-owned xattr under the mount's selected prefix (`trusted.overlay.` or `user.overlay.`); never listed, never escaped, never copied up. | `OverlayXattrType::set_value_on`, `OverlayXattrType::get_value_from` (`inode/xattr.rs`) |
//! | passthrough | Every non-private xattr name, forwarded to the real object unchanged and never interpreted. | `OverlayInode::xattr_name_for_real`, `OverlayInode::xattr_name_for_vfs` (`inode/xattr.rs`) |
//! | escape | The one-segment infix the passthrough path inserts after the selected prefix, so stacked same-prefix overlays physically layer their records. | `OverlayInode::is_escaped_xattr_name`, `OverlayInode::xattr_name_for_real` (`inode/xattr.rs`) |
//! | origin record | The durable layer name plus real inode persisted on an upper object at copy-up, letting the published identity survive the promotion; a non-directory hard link gets none, so its identity then follows its own upper. | `ObjectOriginRecord`, `OverlayFs::record_origin` (`inode/identity.rs`) |
//! | impure marker | The presence-based record on an upper directory holding at least one origin-adjusted (copy-up) entry; its clearing criterion is the origin-preserved summary of a freshly rebuilt snapshot, so the record goes only once no entry is origin-preserved. | `OverlayXattrType::Impure` at each write site (`inode/dir/mod.rs`, `inode/dir/rename.rs`, `inode/copyup/mod.rs`), and the readdir path's best-effort clear (`inode/readdir.rs`) |
//! | readdir cache | The immutable result of one bottom-up merge of a merged directory's layers: index `i` is published with cookie `3 + i`, and the snapshot is shared through `Arc`. | `ReaddirCache` (`inode/readdir.rs`) |
//! | readdir snapshot | One merge result that is immutable once published and `Arc`-shared by every reader that adopts it; a directory keeps its current one in its snapshot slot, which is empty until a rebuild publishes there. The payload's spelling goes through the `OverlayInodeLockPayload` alias, whose name deliberately does not say what the lock carries: a site that takes this lock only to serialize must not read the payload because of it. | `ReaddirCache`, `OverlayInode::current_readdir_cache` (`inode/readdir.rs`), `OverlayInode.lock` (`inode/mod.rs`) |
//! | per-open directory handle | The per-open object a directory open answers with: it owns the snapshot that one open file iterates and the `..` identity captured when the directory was opened, and it publishes into its own slot only to refresh that snapshot. | `OverlayDirOpenHandle`, `OverlayDirOpenHandle::emit_entries` (`inode/open.rs`) |
//! | position cookie | An entry's cookie is its position: `.` is `1`, `..` is `2`, and the entry at index `i` is published with cookie `3 + i`, so a seek lands on the next unconsumed position and a suppressed entry keeps the one it occupies. | `OverlayDirOpenHandle::emit_entries` (`inode/open.rs`) |
//! | deferred ino | An entry whose published inode number cannot come from the layer it won: its snapshot entry carries `0` and the emit path resolves it through the same overlay lookup that `stat` uses, suppressing the entry if that lookup finds nothing. | `ReaddirEntry::overlay_ino` (`inode/readdir.rs`), `OverlayDirOpenHandle::emit_entries` (`inode/open.rs`) |
//! | degrade | The one-shot mount-time warning that an explicitly requested feature is unimplemented; the mount proceeds with the local behavior. | `MountOptions::verify` (`fs/mount/options.rs`) |
//!
//! # Module map
//!
//! | Path | Responsibility |
//! |---|---|
//! | `mod.rs` | Crate entry: registration init. |
//! | `fs_type.rs` | The VFS registration type; answers mount requests on the `overlay` name by constructing `OverlayFs`. |
//! | `layer.rs` | The layer-model types: `Layer`/`LayerStack` and the two-sided per-object `RealObjectStack`. |
//! | `real.rs` | `RealObject`: the dentry-anchored references to underlying objects, the two exits to the real inode and the real dentry, and the real-directory enumeration helpers. |
//! | `fs/mod.rs` | `OverlayFs`: the per-mount state owner and the `FileSystem` trait surface. |
//! | `fs/policy.rs` | `MountPolicy`: the published per-mount decisions (read-only state, xino/uuid modes, xattr prefix, upper capabilities). |
//! | `fs/mount/mod.rs` | The mount construction orchestration (`OverlayFs::new`). |
//! | `fs/mount/options.rs` | Mount option parsing: validation, conflict rejection, degrade warnings. |
//! | `fs/mount/layer_parts.rs` | Mount-time layer assembly: root resolution, overlap and workdir validation, layer-stack build. |
//! | `fs/mount/inuse.rs` | The exclusive upper/workdir claims and the unified overlay identity (uuid). |
//! | `fs/mount/capabilities.rs` | Upper-filesystem capability probes (private xattr, directory entry types, whiteout forms). |
//! | `inode/mod.rs` | `OverlayInode` and `CreateOp`: the logical object, the single create-family request the VFS create entries ride, and the VFS `Inode`/`FileOps` surface. |
//! | `inode/lookup.rs` | Upper-first name resolution and inode projection into the identity-reuse cache. |
//! | `inode/inode_cache.rs` | The identity-reuse cache: one stable identity key to one live `OverlayInode`; a copy-up that persists no origin record moves the promoted instance's entry onto its own upper key, and a lower non-directory hard link keeps one instance per name, held aside from this map. |
//! | `inode/identity.rs` | Dev/ino identity: the mount's resolved form, the layers' numbers, and the durable origin record. |
//! | `inode/readdir.rs` | Merged-directory merge and snapshot construction (the snapshot's fields stay private; enumeration is consumed by `inode/open.rs`). |
//! | `inode/open.rs` | The open entry through the writable take point, and the per-open directory handle that owns one open file's frozen readdir snapshot. |
//! | `inode/data.rs` | Data read/write through the two take points (`O_NOATIME` on lower reads, serialized appends). |
//! | `inode/metadata.rs` | The metadata setters: the six entries, each ending in the writable take point. |
//! | `inode/xattr.rs` | The private-record/passthrough xattr policy, the markers, and the copy-time filter. |
//! | `inode/dir/mod.rs` | The namespace-mutation entries and the parent transaction locks. |
//! | `inode/dir/{create,link,remove,rename}.rs` | The mutation recipes. |
//! | `inode/dir/whiteout.rs` | Whiteout representation, publication, cache, and residue sweeps. |
//! | `inode/copyup/mod.rs` | Copy-up coordination, staging, and atomic publication. |
//! | `inode/copyup/workdir.rs` | The workdir temp lifecycle (create with retry, publish, kind-aware cleanup). |
//!
//! # Reading order
//!
//! 1. The vocabulary table and module map above.
//! 2. `fs/mod.rs` for what one mount owns, then `fs/mount/` for how
//!    construction establishes the mount-time invariants.
//! 3. `layer.rs` and `real.rs` for the type-level foundations every other
//!    module builds on.
//! 4. `inode/mod.rs` and `inode/lookup.rs` for the logical object and how
//!    names resolve onto it.
//! 5. `inode/dir/`, `inode/copyup/`, and `inode/xattr.rs` for mutations,
//!    copy-up, and the private-record policy.
//!
//! # References
//!
//! - Overlay filesystem (kernel documentation):
//!   <https://www.kernel.org/doc/html/latest/filesystems/overlayfs.html>
//! - Concepts and mount options:
//!   <https://elixir.bootlin.com/linux/v7.0/source/Documentation/filesystems/overlayfs.rst#L350-L364>
//! - Layer assembly and overlap checks:
//!   <https://elixir.bootlin.com/linux/v7.0/source/fs/overlayfs/super.c#L1273>
//! - Whiteout creation:
//!   <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/dir.c#L81-L129>
//! - Copy-up on open:
//!   <https://elixir.bootlin.com/linux/latest/source/fs/overlayfs/file.c#L128-L171>

mod fs;
mod fs_type;
mod inode;
mod layer;
mod real;

pub(super) fn init() {
    crate::fs::vfs::registry::register(&fs_type::OverlayFsType).unwrap();
}
