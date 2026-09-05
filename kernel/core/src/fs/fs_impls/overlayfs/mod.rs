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
//! 3. Validate the original resolved paths: same underlying filesystem,
//!    root overlap, workdir-vs-lowers overlap, and instance stability.
//! 4. Build the private clone views for every layer.
//! 5. Re-home the workdir dentry onto the upper layer view mount.
//! 6. Exclusively claim the upper and workdir roots so two overlays cannot
//!    share them.
//! 7. Prepare the staging workspace on writable mounts.
//! 8. Probe what the upper filesystem can store, validate the required
//!    capabilities, and persist the overlay uuid record when effective
//!    (writable mounts only).
//! 9. Publish the mount policy: the fixed per-mount decisions that every
//!    later operation reads.
//!
//! # Vocabulary
//!
//! | Term | Meaning | Code carrier |
//! |---|---|---|
//! | upper / lower | The writable top directory and the read-only directories beneath it; the merge order is fixed at mount time, upper first, lowers topmost-first. | `LayerStack::upper`, `LayerStack::lowers` (`layer.rs`) |
//! | layer | One pinned real directory root of the mount, kept alive by its own private clone view of the underlying filesystem. | `Layer` (`layer.rs`) |
//! | layer identity | The durable pair (underlying container device id, pinned layer-root inode number) that names one layer across copy-up and remount; the mount-local `fsid` is never part of it. | `LayerIdentity`, `Layer::identity` (`layer.rs`) |
//! | layer stack | The ordered, immutable collection of layers assembled once at mount time. | `LayerStack` (`layer.rs`) |
//! | fsid | The per-mount ordinal of one underlying filesystem, shared by every layer rooted on it; encodes layer identity. | `Layer::fsid` (`layer.rs`) |
//! | real object | One underlying filesystem entry as seen from a known layer. | `RealObject` (`real.rs`) |
//! | real-object stack | The complete real-object composition behind one logical object: an optional upper object plus the retained lower objects. | `RealObjectStack` (`layer.rs`) |
//! | visible source | The topmost real object of a stack — the upper when present, else the topmost lower; it provides the visible metadata and the cache key. | `RealObjectStack::visible_source` (`layer.rs`) |
//! | merged directory | A directory whose visible names unite upper and lower contributions. | `RealObjectStack::is_merged` (`layer.rs`) |
//! | logical object | The overlay inode published to the VFS, shared by every name bound to the same real-object stack. | `OverlayInode` (`inode/mod.rs`) |
//! | projection | Creating or reusing the shared logical inode for a real-object stack, and computing what it reports. | `OverlayFs::project_inode` (`inode/lookup.rs`) |
//! | published identity | The precomputed `st_dev`/`st_ino` pair a logical object reports, kept stable across copy-up; the `project` xino matrix serves live objects and the retained-origin path (`project_origin_object_id`) serves copied-up ones. | `ObjectId` (`inode/identity.rs`) |
//! | xino | The encoding that packs a layer's fsid into the high bits of the published inode number; its effective mode is resolved once at mount into `is_same_fs_passthrough`/`is_xino_effective`, and `xino=on` forces it even on one filesystem. | `IdentityPolicy::XINO_SHIFT`, `IdentityPolicy::project`, `OverlayFs::project_origin_object_id` (`inode/identity.rs`) |
//! | real-object key | The identity pair (layer fsid plus real inode number of the visible source) under which the identity-reuse cache shares one logical object. | `RealObjectKey` (`real.rs`), `InodeCache` (`inode/inode_cache.rs`) |
//! | claim | The mount-time exclusive lease on the upper and workdir root inodes; a second overlay claiming either is refused. | `UpperWorkdirInuse` (`fs/mount/inuse.rs`) |
//! | workdir | The claimed directory on the upper's filesystem; its prepared `<workdir>/work` staging workspace hosts all private staging objects. | `UpperWorkdirInuse::prepare_workdir` (`fs/mount/inuse.rs`) |
//! | workdir temp | A staged private object created in the staging workspace, later published by rename or cleaned up by kind. | `WorkdirTemp`, `OverlayFs::create_workdir_temp` (`inode/copyup/workdir.rs`) |
//! | copy-up | The lower-to-upper promotion of a written object: stage a private workdir temp, then atomically rename it into place, ancestors first. | `OverlayInode::copy_up_at` (`inode/copyup/mod.rs`) |
//! | publication coordinate | The (publication parent, name) pair a copy-up publishes at, sourced from the operation's overlay dentry's parent and name. | `OverlayInode::publication_coordinate` (`inode/copyup/mod.rs`) |
//! | anchor path | The layer-relative path of an object's visible-source real dentry, collected up to its layer root; re-walking it from the mount root re-resolves the object's current overlay position for the readdir `..` entry. | `OverlayInode::anchor_path`, `OverlayFs::resolve_at_anchor` (`inode/lookup.rs`) |
//! | admission | The permission gates of every request: the read-only gate (`check_permission`, never promotes) and the mutating gate (`check_mutating_permission`: the local check, copy-up promotion, then the real-handle re-check). | `OverlayInode::check_permission`, `OverlayInode::check_mutating_permission`, `AccessType` (`inode/permission.rs`) |
//! | per-inode transaction lock | The per-inode mutex that serializes one object's mutations; directories run their namespace-mutation recipes under it and carry the readdir cache in its payload. | `OverlayInode.lock`, `OverlayInode::lock` (`inode/mod.rs`, `inode/dir/mod.rs`) |
//! | recipe | One namespace-mutation procedure (create, link, remove, rename) that runs under the parent's transaction lock after admission. | `inode/dir/{create,link,remove,rename}.rs` |
//! | whiteout | A name-level visibility barrier published in the upper to hide a lower-backed name; either a char device `0:0` or a marked regular file. | `OverlayFs::publish_whiteout` (`inode/dir/whiteout.rs`), `is_whiteout_inode` (`inode/lookup.rs`) |
//! | whiteout cache | The mount-time shared whiteout handle plus the one-way link-capability latch; a publish links from the shared handle or renames a one-shot workdir temp. | `WhiteoutCache` (`inode/dir/whiteout.rs`) |
//! | opaque directory | A directory-level barrier: a real directory whose private record cuts off every lower contribution beneath it. | `is_opaque_directory` (`inode/lookup.rs`) |
//! | stale upper | An upper-backed record whose fresh layer truth no longer resolves is surfaced as `ESTALE` by translating the real-layer `ENOENT`; there is no proactive stale detection and no cache rebuild. | `translate_stale_upper_enoent` (`inode/dir/remove.rs`) |
//! | private record | An overlay-owned xattr under the mount's selected prefix (`trusted.overlay.` or `user.overlay.`); never listed, never escaped, never copied up. | `OverlayRecordName::construct_xattr_name` (`inode/xattr.rs`) |
//! | passthrough | Every non-private xattr name, forwarded to the real object unchanged and never interpreted. | `used_full_name`, `present_xattr_names` (`inode/xattr.rs`) |
//! | escape | The one-segment infix the passthrough path inserts after the selected prefix, so stacked same-prefix overlays physically layer their records. | `ESCAPE_INFIX`, `used_full_name` (`inode/xattr.rs`) |
//! | lower-id record | The durable layer identity plus real inode persisted on an upper object at copy-up, letting the published identity survive the promotion. | `LowerIdOrigin` (`inode/identity.rs`), `LayerIdentity` (`layer.rs`) |
//! | impure marker | The presence-based record on an upper directory holding at least one origin-adjusted (copy-up) entry; the durable mirror of the readdir impure-entry predicate. | `OverlayInode::set_impure_marker` (`inode/xattr.rs`) |
//! | readdir cache | The stable, resumable name-to-cookie cache of a merged directory; each visible entry carries its published identity and cookies are monotonic and never reused. | `ReaddirCache` (`inode/readdir.rs`) |
//! | tombstone | A deleted-name placeholder in the readdir cache that keeps its cookie so already-exposed positions stay stable. | `ReaddirCacheEntry::Tombstone` (`inode/readdir.rs`) |
//! | degrade | The one-shot mount-time disclosure that an explicitly requested feature is unimplemented; the mount proceeds with the local behavior. | `MountOptions::verify` (`fs/mount/options.rs`) |
//!
//! # Module map
//!
//! | Path | Responsibility |
//! |---|---|
//! | `mod.rs` | Crate entry: registration init plus the shared child-name read helper. |
//! | `fs_type.rs` | The VFS registration type; answers mount requests on the `overlay` name by constructing `OverlayFs`. |
//! | `layer.rs` | The layer-model types: `Layer`/`LayerIdentity`/`LayerStack` and the per-object `RealObjectStack`. |
//! | `real.rs` | `RealObject`/`RealObjectKey`: dentry-anchored references to underlying objects. |
//! | `fs/mod.rs` | `OverlayFs`: the per-mount state owner and the `FileSystem` trait surface. |
//! | `fs/policy.rs` | `MountPolicy`: the published per-mount decisions (read-only state, permission mode, xino/uuid modes, xattr prefix). |
//! | `fs/mount/mod.rs` | The mount construction orchestration (`OverlayFs::new`). |
//! | `fs/mount/options.rs` | Mount option parsing: validation, conflict rejection, degrade disclosure. |
//! | `fs/mount/layer_parts.rs` | Mount-time layer assembly: root resolution, overlap and workdir validation, layer-stack build. |
//! | `fs/mount/inuse.rs` | The exclusive upper/workdir claims and the unified overlay identity (uuid). |
//! | `fs/mount/capabilities.rs` | Upper-filesystem capability probes (private xattr, directory entry types, whiteout forms). |
//! | `inode/mod.rs` | `OverlayInode`: the logical object and the VFS `Inode`/`FileOps` surface. |
//! | `inode/lookup.rs` | Upper-first name resolution and inode projection into the identity-reuse cache. |
//! | `inode/inode_cache.rs` | The identity-reuse cache: one real-object key to one live `OverlayInode`. |
//! | `inode/identity.rs` | Published dev/ino identity: the xino matrix, layer-resolution consumers, and origin records. |
//! | `inode/readdir.rs` | The merged-directory readdir cache and enumeration service. |
//! | `inode/data.rs` | Data read/write delegation (`O_NOATIME` on lower reads, serialized appends). |
//! | `inode/permission.rs` | The admission pipeline and the shared credential probes. |
//! | `inode/metadata.rs` | The metadata setters and their ownership gate. |
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

#![short_vis_path::add(overlayfs)]

mod fs;
mod fs_type;
mod inode;
mod layer;
mod real;

use ostd::task::{CurrentTask, Task};

use crate::{
    fs::vfs::{inode::Inode, path},
    prelude::*,
    process::posix_thread::{AsPosixThread, PosixThread},
};

pub(in overlayfs) fn with_current_posix_thread<T>(
    operation_fn: impl FnOnce(&CurrentTask, &PosixThread) -> T,
) -> Option<T> {
    let task = Task::current()?;
    let posix_thread = task.as_posix_thread()?;
    Some(operation_fn(&task, posix_thread))
}

pub(in overlayfs) fn read_child_names(real_dir: &Arc<dyn Inode>) -> Result<Vec<String>> {
    let mut names: Vec<String> = Vec::new();
    let mut offset = 0;
    loop {
        match real_dir.readdir_at(offset, &mut names)? {
            0 => break,
            visited => offset += visited,
        }
    }
    names.retain(|name| !path::is_dot_or_dotdot(name));
    Ok(names)
}

pub(super) fn init() {
    crate::fs::vfs::registry::register(&fs_type::OverlayFsType).unwrap();
}
