// SPDX-License-Identifier: MPL-2.0
#![short_vis_path::add(overlayfs)]

//! The merged-directory readdir index and enumeration service.
//!
//! A merged overlay directory must iterate its visible names in a stable,
//! resumable order, so each overlay directory keeps one [`ReaddirIndex`].
//!
//! ## Index contract
//!
//! The index is the first source for visible names: exactly one current
//! [`ReaddirIndex`] exists per overlay directory (`Some` iff directory);
//! cookies are monotonic and never reused, with `1`/`2` reserved for `.`/`..`.
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

use hashbrown::HashSet;

use super::{Lookup, OverlayInode, identity::ObjectId, lookup::is_opaque_directory};
use crate::{
    fs::{
        file::InodeType,
        fs_impls::overlayfs::{fs::OverlayFs, layer::RealObjectStack, real::RealObject},
        utils::DirentVisitor,
        vfs::inode::Inode,
    },
    prelude::*,
};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct ReaddirCookie(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReaddirIndexValidity {
    Valid,
    NeedsRebuild,
}

pub(super) struct ReaddirIndex {
    entries: Vec<ReaddirIndexEntry>,
    validity: ReaddirIndexValidity,
    next_cookie: ReaddirCookie,
    tombstone_count: usize,
}

enum ReaddirIndexEntry {
    Visible {
        name: String,
        cookie: ReaddirCookie,
        inode: Arc<OverlayInode>,
        type_: InodeType,
    },
    Tombstone {
        name: String,
        cookie: ReaddirCookie,
        inode: Weak<OverlayInode>,
    },
}

impl OverlayInode {
    pub(super) fn readdir_at_impl(
        &self,
        offset: usize,
        visitor: &mut dyn DirentVisitor,
    ) -> Result<usize> {
        let mut lock = self.lock.lock();
        let index = lock.as_mut().ok_or_else(|| Error::new(Errno::ENOTDIR))?;
        let facts = self.real_object_stack();
        let input_cookie = ReaddirCookie(offset as u64);
        self.ensure_readdir_index(&facts, index)?;
        let mut last_visited: Option<ReaddirCookie> = None;
        let delta_fn = |last_visited: Option<ReaddirCookie>| -> usize {
            let delta = match last_visited {
                Some(last) => last.0 - input_cookie.0,
                None => 0,
            };
            usize::try_from(delta).unwrap_or(usize::MAX)
        };
        if input_cookie < ReaddirCookie(1) {
            visitor.visit(".", self.ino(), InodeType::Dir, 1)?;
            last_visited = Some(ReaddirCookie(1));
        }
        if input_cookie < ReaddirCookie(2) {
            let parent_object_id = self.resolve_parent_object_id();
            if visitor
                .visit("..", parent_object_id.ino, InodeType::Dir, 2)
                .is_err()
            {
                // `.` was already consumed by this call, so the consumed
                // delta is returned instead of propagating the error.
                return Ok(delta_fn(last_visited));
            }
            last_visited = Some(ReaddirCookie(2));
        }
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
                continue;
            };
            let d_off = match usize::try_from(cookie.0) {
                Ok(d_off) => d_off,
                Err(_) => break,
            };
            if let Err(err) = visitor.visit(name, inode.ino(), *type_, d_off) {
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
    pub(super) fn invalidate_readdir_index(&self, index: &mut Option<ReaddirIndex>) {
        if let Some(index) = index.as_mut() {
            index.validity = ReaddirIndexValidity::NeedsRebuild;
        }
    }

    /// A merged/lower-backed or stale index cannot provably keep cookie
    /// order, so only a `Valid` upper-only index is updated in place.
    pub(super) fn readdir_index_insert(
        &self,
        name: &str,
        inode: Arc<OverlayInode>,
        type_: InodeType,
        index: &mut Option<ReaddirIndex>,
    ) {
        // Always record the freshly visible inode so the remove path can
        // later detect a stale-upper disappearance even in parents whose
        // index no prior readdir has built.
        let index = index.get_or_insert_with(ReaddirIndex::new);
        if !index.insert_visible(name, inode, type_)
            || index.validity != ReaddirIndexValidity::Valid
            || self.upper.get().is_none()
            || !self.lowers.is_empty()
        {
            index.validity = ReaddirIndexValidity::NeedsRebuild;
        }
    }

    pub(super) fn readdir_index_remove(&self, name: &str, index: &mut Option<ReaddirIndex>) {
        let Some(index) = index.as_mut() else {
            return;
        };
        if index.validity == ReaddirIndexValidity::Valid && !index.remove_visible(name) {
            index.validity = ReaddirIndexValidity::NeedsRebuild;
        }
    }

    pub(super) fn finish_whiteout_index(
        &self,
        name: Option<&str>,
        index: &mut Option<ReaddirIndex>,
    ) {
        match name {
            Some(name) => self.readdir_index_remove(name, index),
            None => self.invalidate_readdir_index(index),
        }
    }

    /// Acquires the directory's own `lock`; a caller already holding it
    /// must use `ensure_readdir_index` directly.
    pub(super) fn visible_child_count(&self) -> Result<usize> {
        let mut lock = self.lock.lock();
        let index = lock.as_mut().ok_or_else(|| {
            Error::with_message(Errno::ENOTDIR, "the overlay inode is not a directory")
        })?;
        let facts = self.real_object_stack();
        self.ensure_readdir_index(&facts, index)?;
        Ok(index
            .entries
            .iter()
            .filter(|entry| matches!(entry, ReaddirIndexEntry::Visible { .. }))
            .count())
    }

    pub(super) fn ensure_readdir_index(
        &self,
        facts: &RealObjectStack,
        index: &mut ReaddirIndex,
    ) -> Result<()> {
        if index.validity == ReaddirIndexValidity::Valid {
            return Ok(());
        }
        // Directory copy-up preserves the visible sequence (empty upper,
        // retained lowers), so no post-scan facts revalidation is needed.
        let sequence = self.readdir_sequence(facts)?;
        index.rebuild(sequence);
        Ok(())
    }

    fn readdir_sequence(
        &self,
        facts: &RealObjectStack,
    ) -> Result<Vec<(String, Arc<OverlayInode>, InodeType)>> {
        let fs = self.fs();
        let fs = fs.downcast_ref::<OverlayFs>().ok_or_else(|| {
            Error::with_message(
                Errno::EIO,
                "the overlay inode is not backed by an overlay mount",
            )
        })?;
        let layers: Vec<&RealObject> = if !facts.is_merged() {
            let source = match facts.upper.as_ref() {
                Some(upper) => upper,
                None => &facts.lowers[0],
            };
            vec![source]
        } else {
            let prefix = fs.policy().xattr_prefix();
            let mut layers = Vec::new();
            for layer in facts.upper.iter().chain(facts.lowers.iter()) {
                layers.push(layer);
                if is_opaque_directory(layer, prefix)? {
                    break;
                }
            }
            layers
        };
        let mut seen = HashSet::new();
        let mut sequence = Vec::new();
        for layer in layers {
            for name in crate::fs::fs_impls::overlayfs::read_child_names(layer.real_inode())? {
                if !seen.insert(name.clone()) {
                    continue;
                }
                if let Lookup::Positive(inode) = fs.lookup(self, &name)? {
                    let file_type = inode.type_();
                    sequence.push((name, inode, file_type));
                }
            }
        }
        Ok(sequence)
    }
}

impl OverlayInode {
    /// The `..` identity: the exact parent id re-resolved at the object's
    /// anchor path (everything but the last component); any anchor-path
    /// failure — a corrupt anchor from `anchor_path`, or a failed
    /// re-resolution — degrades to the object's own id. An empty anchor is
    /// the mount root, which reports itself. Never panics: the mount arm
    /// is defensive and unreachable for a live overlay inode.
    fn resolve_parent_object_id(&self) -> ObjectId {
        let Ok(fs) = self.fs_arc() else {
            return self.object_id();
        };
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

impl ReaddirIndex {
    pub(super) fn new() -> Self {
        Self {
            entries: Vec::new(),
            validity: ReaddirIndexValidity::NeedsRebuild,
            next_cookie: ReaddirCookie(3),
            tombstone_count: 0,
        }
    }

    pub(super) fn visible_inodes(&self) -> Vec<Arc<OverlayInode>> {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                ReaddirIndexEntry::Visible { inode, .. } => Some(inode.clone()),
                ReaddirIndexEntry::Tombstone { .. } => None,
            })
            .collect()
    }

    /// The per-name record used to detect an upper-backed name whose
    /// upper object no longer resolves (a stale-upper disappearance).
    pub(super) fn visible_inode(&self, name: &str) -> Option<Arc<OverlayInode>> {
        self.entries.iter().find_map(|entry| match entry {
            ReaddirIndexEntry::Visible {
                name: entry_name,
                inode,
                ..
            } if entry_name == name => Some(inode.clone()),
            _ => None,
        })
    }

    /// A name that was `Visible` before, denotes the same logical object,
    /// and kept a cookie above `last_assigned` retains its cookie; every
    /// other appearance gets a fresh one.
    fn rebuild(&mut self, sequence: Vec<(String, Arc<OverlayInode>, InodeType)>) {
        let mut entries = Vec::with_capacity(sequence.len());
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
                    // Cookie exhaustion is unreachable for a real directory;
                    // saturating keeps cookie order monotonic.
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

    #[must_use]
    fn remove_visible(&mut self, name: &str) -> bool {
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
        if self.tombstone_count >= self.entries.len() - self.tombstone_count {
            self.compact_tombstones();
        }
        true
    }

    /// The create path is only for names proven to belong at the end of the
    /// cookie order; a mid-sequence insert must fall back to `NeedsRebuild`
    /// — never renumber already-exposed cookies.
    #[must_use]
    fn insert_visible(&mut self, name: &str, inode: Arc<OverlayInode>, type_: InodeType) -> bool {
        if let Some(index) = self.entries.iter().position(|entry| {
            matches!(
                entry,
                ReaddirIndexEntry::Tombstone { name: entry_name, .. } if entry_name == name
            )
        }) {
            let revive = match &self.entries[index] {
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
        }
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

    fn compact_tombstones(&mut self) {
        self.entries
            .retain(|entry| matches!(entry, ReaddirIndexEntry::Visible { .. }));
        self.tombstone_count = 0;
    }
}
