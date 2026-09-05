// SPDX-License-Identifier: MPL-2.0

//! The merged-directory readdir cache and enumeration service.
//!
//! A merged overlay directory must iterate its visible names in a stable,
//! resumable order, so each overlay directory keeps one [`ReaddirCache`].
//!
//! ## Cache contract
//!
//! The cache is the first source for visible names: exactly one current
//! [`ReaddirCache`] exists per overlay directory (`Some` iff directory);
//! cookies are monotonic and never reused, with `1`/`2` reserved for `.`/`..`.
//! Every visible entry carries the `published_ino` that `stat`/lookup reports
//! and whether it is origin-preserved (`is_impure`), so a cache hit never
//! re-materializes the child and the cache owns the marker-clear predicate.
//!
//! A **`Tombstone`** entry records a deleted name that keeps its cookie. An
//! **opaque directory** is a lower-search barrier: the opaque layer's own
//! names still surface, but names in the layers below it never do.
//!
//! ## `..` identity
//!
//! The `..` entry carries the overlay-parent identity re-derived from the
//! object's anchor path ([`OverlayInode::resolve_parent_object_id`]); when
//! the anchor no longer resolves, it degrades to the object's own identity.

#![short_vis_path::add(overlayfs)]

use hashbrown::HashMap;

use super::{
    OverlayInode,
    identity::ObjectId,
    lookup::{is_opaque_directory, is_whiteout_inode},
};
use crate::{
    fs::{
        file::InodeType,
        fs_impls::overlayfs::{fs::OverlayFs, real::RealObject},
        utils::DirentVisitor,
        vfs::inode::Inode,
    },
    prelude::*,
};

type ReaddirCookie = u64;

pub(super) struct ReaddirCache {
    entries: Vec<ReaddirCacheEntry>,
    is_valid: bool,
    next_cookie: ReaddirCookie,
    tombstone_count: usize,
}

enum ReaddirCacheEntry {
    Visible {
        name: String,
        type_: InodeType,
        published_ino: u64,
        cookie: ReaddirCookie,
        /// Whether the published identity is origin-preserved.
        is_impure: bool,
    },
    Tombstone {
        name: String,
        published_ino: u64,
        cookie: ReaddirCookie,
    },
}

/// One first-seen directory name and the merge facts recorded for it.
enum ReaddirScanEntry {
    /// A child of the merged directory that readdir publishes.
    VisibleChild {
        name: String,
        type_: InodeType,
        /// The direct identity, possibly overridden for origin-preservation by `finish`.
        published_ino: u64,
        /// The upper source; present iff the first-seen layer is layer `0`.
        upper: Option<RealObject>,
        /// Mirrors the `lookup_in_layers` lower directory hits.
        lowers: Vec<RealObject>,
        /// An opaque upper-directory winner or opaque duplicate stops lower collection.
        stop_lowers: bool,
    },
    /// A whiteout occupying the name; `finish` filters it out.
    Whiteout,
}

/// The scan's first-seen entries plus the `name -> index` table; `finish` consumes it.
struct ReaddirScan {
    entries: Vec<ReaddirScanEntry>,
    index_by_name: HashMap<String, usize>,
}

impl ReaddirScan {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            index_by_name: HashMap::new(),
        }
    }

    /// The only mutable by-name view of a recorded entry.
    fn entry_mut(&mut self, name: &str) -> Option<&mut ReaddirScanEntry> {
        let index = *self.index_by_name.get(name)?;
        self.entries.get_mut(index)
    }

    /// The only append path: grows both containers in one step.
    fn insert(&mut self, name: &str, entry: ReaddirScanEntry) {
        debug_assert!(match &entry {
            ReaddirScanEntry::VisibleChild { name: inner, .. } => inner.as_str() == name,
            ReaddirScanEntry::Whiteout => true,
        });
        let index = self.entries.len();
        self.index_by_name.insert(name.to_string(), index);
        self.entries.push(entry);
    }

    /// The closing pass: origin resolution, whiteout filtering, and cookie assignment.
    fn finish(self, fs: &OverlayFs, previous: &ReaddirCache) -> ReaddirCache {
        let mut entries = Vec::with_capacity(self.entries.len());
        let mut next_cookie = previous.next_cookie;
        let mut last_assigned = 2;
        for entry in self.entries {
            let ReaddirScanEntry::VisibleChild {
                name,
                type_,
                published_ino,
                upper,
                lowers,
                ..
            } = entry
            else {
                continue; // ReaddirScanEntry::Whiteout: suppressed, never published
            };
            let ResolvedIdentity {
                published_ino,
                is_impure,
            } = resolve_visible_identity(fs, type_, upper.as_ref(), &lowers, published_ino);
            let previous_cookie = previous.entries.iter().find_map(|old| match old {
                ReaddirCacheEntry::Visible {
                    name: old_name,
                    published_ino: old_ino,
                    cookie,
                    ..
                } if old_name == &name && *old_ino == published_ino => Some(*cookie),
                _ => None,
            });
            let cookie = match previous_cookie {
                Some(previous) if previous > last_assigned => previous,
                _ => {
                    let fresh = next_cookie;
                    next_cookie = next_cookie.saturating_add(1);
                    fresh
                }
            };
            last_assigned = cookie;
            entries.push(ReaddirCacheEntry::Visible {
                name,
                type_,
                published_ino,
                cookie,
                is_impure,
            });
        }
        ReaddirCache {
            entries,
            is_valid: true,
            next_cookie,
            tombstone_count: 0,
        }
    }
}

/// The resolved readdir identity of one visible child.
#[derive(Clone, Copy)]
struct ResolvedIdentity {
    published_ino: u64,
    is_impure: bool,
}

/// Resolves the identity readdir must publish so enumeration and lookup agree.
fn resolve_visible_identity(
    fs: &OverlayFs,
    type_: InodeType,
    upper: Option<&RealObject>,
    lowers: &[RealObject],
    direct_ino: u64,
) -> ResolvedIdentity {
    let Some(upper) = upper else {
        return ResolvedIdentity {
            published_ino: direct_ino,
            is_impure: false,
        };
    };
    // A live instance for the same real object wins; its retained lowers decide origin.
    if let Some(carrier) = fs.inodes().get(fs.real_object_key(upper))
        && Arc::ptr_eq(carrier.visible_source().real_inode(), upper.real_inode())
    {
        return ResolvedIdentity {
            published_ino: carrier.ino(),
            is_impure: !carrier.lowers.is_empty(),
        };
    }
    // With no live instance, a directory winner with same-name lowers derives its origin record.
    if !lowers.is_empty() {
        match fs.project_origin_object_id(upper.real_inode(), lowers, type_.is_directory()) {
            Ok(Some(object_id)) => {
                return ResolvedIdentity {
                    published_ino: object_id.ino,
                    is_impure: true,
                };
            }
            Ok(None) => {}
            Err(err) => warn!(
                "failed to read the lower-id record of the upper source; \
                 falling back to the visible-source projection: {:?}",
                err
            ),
        }
    }
    ResolvedIdentity {
        published_ino: direct_ino,
        is_impure: false,
    }
}

impl OverlayInode {
    pub(super) fn readdir_at_impl(
        &self,
        offset: usize,
        visitor: &mut dyn DirentVisitor,
    ) -> Result<usize> {
        let mut lock = self.lock();
        let index = lock.as_mut().ok_or_else(|| Error::new(Errno::ENOTDIR))?;
        let input_cookie = offset as u64;
        self.ensure_readdir_cache(index)?;
        let mut last_visited: Option<ReaddirCookie> = None;
        let delta_fn = |last_visited: Option<ReaddirCookie>| -> usize {
            let delta = match last_visited {
                Some(last) => last - input_cookie,
                None => 0,
            };
            usize::try_from(delta).unwrap_or(usize::MAX)
        };
        if input_cookie < 1 {
            visitor.visit(".", self.ino(), InodeType::Dir, 1)?;
            last_visited = Some(1);
        }
        if input_cookie < 2 {
            let parent_object_id = self.resolve_parent_object_id();
            if visitor
                .visit("..", parent_object_id.ino, InodeType::Dir, 2)
                .is_err()
            {
                // `.` was already consumed; return its delta instead of the error.
                return Ok(delta_fn(last_visited));
            }
            last_visited = Some(2);
        }
        let start = index
            .first_entry_after(input_cookie)
            .unwrap_or(index.entries.len());
        for entry in &index.entries[start..] {
            let ReaddirCacheEntry::Visible {
                name,
                cookie,
                published_ino,
                type_,
                ..
            } = entry
            else {
                continue;
            };
            let d_off = match usize::try_from(*cookie) {
                Ok(d_off) => d_off,
                Err(_) => break,
            };
            if let Err(err) = visitor.visit(name, *published_ino, *type_, d_off) {
                if last_visited.is_none() {
                    return Err(err);
                }
                break;
            }
            last_visited = Some(*cookie);
        }
        Ok(delta_fn(last_visited))
    }
}

impl OverlayInode {
    pub(super) fn invalidate_readdir_cache(&self, index: &mut Option<ReaddirCache>) {
        if let Some(index) = index.as_mut() {
            index.is_valid = false;
        }
    }

    /// Only a valid upper-only cache is updated in place; other caches cannot keep order.
    pub(super) fn readdir_cache_insert(
        &self,
        name: &str,
        type_: InodeType,
        published_ino: u64,
        is_impure: bool,
        index: &mut Option<ReaddirCache>,
    ) {
        let index = index.get_or_insert_with(ReaddirCache::new);
        if !index.insert_visible(name, type_, published_ino, is_impure)
            || !index.is_valid
            || self.upper.get().is_none()
            || !self.lowers.is_empty()
        {
            index.is_valid = false;
        }
    }

    pub(super) fn readdir_cache_remove(&self, name: &str, index: &mut Option<ReaddirCache>) {
        let Some(index) = index.as_mut() else {
            return;
        };
        if index.is_valid && !index.remove_visible(name) {
            index.is_valid = false;
        }
    }

    pub(super) fn finish_whiteout_cache(
        &self,
        name: Option<&str>,
        index: &mut Option<ReaddirCache>,
    ) {
        match name {
            Some(name) => self.readdir_cache_remove(name, index),
            None => self.invalidate_readdir_cache(index),
        }
    }

    /// Acquires the directory lock; a caller already holding it must use `ensure_readdir_cache`.
    pub(super) fn visible_child_count(&self) -> Result<usize> {
        let mut lock = self.lock();
        let index = lock.as_mut().ok_or_else(|| {
            Error::with_message(Errno::ENOTDIR, "the overlay inode is not a directory")
        })?;
        self.ensure_readdir_cache(index)?;
        Ok(index.entries.len().saturating_sub(index.tombstone_count))
    }

    pub(super) fn ensure_readdir_cache(&self, index: &mut ReaddirCache) -> Result<()> {
        if index.is_valid {
            return Ok(());
        }
        // Bind the rebuilt value first: the borrow of `index` ends before installation.
        let cache = self.build_readdir_cache(index)?;
        *index = cache;
        Ok(())
    }

    fn build_readdir_cache(&self, previous: &ReaddirCache) -> Result<ReaddirCache> {
        let fs = self.fs();
        let fs = fs.downcast_ref::<OverlayFs>().ok_or_else(|| {
            Error::with_message(
                Errno::EIO,
                "the overlay inode is not backed by an overlay mount",
            )
        })?;
        let namespace = fs.policy().xattr_namespace();
        let is_merged =
            (self.upper.get().is_some() && !self.lowers.is_empty()) || self.lowers.len() > 1;
        let layers: Vec<&RealObject> = if !is_merged {
            vec![self.visible_source()]
        } else {
            let mut layers = Vec::new();
            for layer in self.upper.get().into_iter().chain(self.lowers.iter()) {
                layers.push(layer);
                if is_opaque_directory(layer, namespace)? {
                    break;
                }
            }
            layers
        };

        let mut scan = ReaddirScan::new();
        for layer in layers {
            let layer_index = layer.layer_index();
            let real_layer = fs.layer(layer_index);
            let dir = layer.dentry().as_dir_dentry_or_err()?;
            for name in crate::fs::fs_impls::overlayfs::read_child_names(layer.real_inode())? {
                if let Some(entry) = scan.entry_mut(name.as_str()) {
                    match entry {
                        ReaddirScanEntry::Whiteout => continue,
                        ReaddirScanEntry::VisibleChild {
                            stop_lowers,
                            lowers,
                            ..
                        } => {
                            if *stop_lowers {
                                continue;
                            }
                            let child = match dir.lookup_child(name.as_str()) {
                                Ok(child) => child,
                                Err(err) if err.error() == Errno::ENOENT => continue,
                                Err(err) => return Err(err),
                            };
                            if !child.inode().type_().is_directory() {
                                *stop_lowers = true;
                                continue;
                            }
                            let lower = RealObject::new(layer_index, child);
                            if is_opaque_directory(&lower, namespace)? {
                                *stop_lowers = true;
                            }
                            lowers.push(lower);
                        }
                    }
                    continue;
                }
                let child = match dir.lookup_child(name.as_str()) {
                    Ok(child) => child,
                    Err(err) if err.error() == Errno::ENOENT => continue,
                    Err(err) => return Err(err),
                };
                if is_whiteout_inode(child.inode(), namespace)? {
                    scan.insert(name.as_str(), ReaddirScanEntry::Whiteout);
                    continue;
                }
                let type_ = child.inode().type_();
                let is_dir = type_.is_directory();
                let published_ino = fs
                    .identity()
                    .project(
                        real_layer.fsid,
                        child.inode().ino(),
                        real_layer.container_dev_id,
                        is_dir,
                    )
                    .ino;
                let is_upper_dir_winner = layer_index == 0 && is_dir;
                let upper = (layer_index == 0).then(|| RealObject::new(layer_index, child.clone()));
                // An opaque upper directory is a lower-search barrier.
                let stop_lowers = match &upper {
                    Some(upper) if is_upper_dir_winner => is_opaque_directory(upper, namespace)?,
                    _ => !is_upper_dir_winner,
                };
                scan.insert(
                    name.as_str(),
                    ReaddirScanEntry::VisibleChild {
                        name: name.clone(),
                        type_,
                        published_ino,
                        upper,
                        lowers: Vec::new(),
                        stop_lowers,
                    },
                );
            }
        }
        Ok(scan.finish(fs, previous))
    }
}

impl OverlayInode {
    /// The `..` identity: the parent id re-resolved at the anchor path, else the object's id.
    fn resolve_parent_object_id(&self) -> ObjectId {
        let fs = self.fs_arc();
        let Ok(anchor) = self.anchor_path(&fs) else {
            return self.object_id();
        };
        if anchor.is_empty() {
            return self.object_id();
        }
        match fs.resolve_at_anchor(&anchor[..anchor.len() - 1]) {
            Ok(parent) => parent.object_id(),
            Err(_) => self.object_id(),
        }
    }
}

impl ReaddirCache {
    pub(super) fn new() -> Self {
        Self {
            entries: Vec::new(),
            is_valid: false,
            next_cookie: 3,
            tombstone_count: 0,
        }
    }

    /// Whether any visible entry keeps this directory impure, gating the marker clear.
    pub(super) fn has_impure_entry(&self) -> bool {
        self.entries.iter().any(|entry| {
            matches!(
                entry,
                ReaddirCacheEntry::Visible {
                    is_impure: true,
                    ..
                }
            )
        })
    }

    fn first_entry_after(&self, cookie: ReaddirCookie) -> Option<usize> {
        let index = self.entries.partition_point(|entry| match entry {
            ReaddirCacheEntry::Visible {
                cookie: entry_cookie,
                ..
            }
            | ReaddirCacheEntry::Tombstone {
                cookie: entry_cookie,
                ..
            } => *entry_cookie <= cookie,
        });
        (index < self.entries.len()).then_some(index)
    }

    #[must_use]
    fn remove_visible(&mut self, name: &str) -> bool {
        let Some(index) = self.entries.iter().position(|entry| {
            matches!(
                entry,
                ReaddirCacheEntry::Visible { name: entry_name, .. } if entry_name == name
            )
        }) else {
            return false;
        };
        let (name, cookie, published_ino) = match &self.entries[index] {
            ReaddirCacheEntry::Visible {
                name,
                cookie,
                published_ino,
                ..
            } => (name.clone(), *cookie, *published_ino),
            _ => return false,
        };
        self.entries[index] = ReaddirCacheEntry::Tombstone {
            name,
            published_ino,
            cookie,
        };
        self.tombstone_count += 1;
        if self.tombstone_count >= self.entries.len() - self.tombstone_count {
            self.compact_tombstones();
        }
        true
    }

    /// Only names at the end of the cookie order may be added; otherwise invalidate the cache.
    #[must_use]
    fn insert_visible(
        &mut self,
        name: &str,
        type_: InodeType,
        published_ino: u64,
        is_impure: bool,
    ) -> bool {
        if let Some(index) = self.entries.iter().position(|entry| {
            matches!(
                entry,
                ReaddirCacheEntry::Tombstone { name: entry_name, .. } if entry_name == name
            )
        }) {
            let revive = match &self.entries[index] {
                ReaddirCacheEntry::Tombstone {
                    name,
                    cookie,
                    published_ino: old_ino,
                } if *old_ino == published_ino => Some((name.clone(), *cookie)),
                _ => None,
            };
            if let Some((name, cookie)) = revive {
                self.entries[index] = ReaddirCacheEntry::Visible {
                    name,
                    cookie,
                    published_ino,
                    is_impure,
                    type_,
                };
                self.tombstone_count -= 1;
                return true;
            }
        }
        let cookie = self.next_cookie;
        self.next_cookie = self.next_cookie.saturating_add(1);
        self.entries.push(ReaddirCacheEntry::Visible {
            name: name.into(),
            cookie,
            published_ino,
            is_impure,
            type_,
        });
        false
    }

    fn compact_tombstones(&mut self) {
        self.entries
            .retain(|entry| matches!(entry, ReaddirCacheEntry::Visible { .. }));
        self.tombstone_count = 0;
    }
}
