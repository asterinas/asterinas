// SPDX-License-Identifier: MPL-2.0

//! The merged-directory snapshot builder and its enumeration service.
//!
//! A merged directory answers `readdir` from one **snapshot**: the immutable result of a single
//! bottom-up merge over the layers `lookup` already materialized. The snapshot lives in the
//! directory's transaction-lock payload, which holds `None` while no current snapshot exists;
//! a mutation that changes the visible name set clears that payload, so the next read merges
//! again. Nothing here resolves a child's identity or re-reads opaqueness: a layer's own
//! enumeration answer supplies each entry's type and inode number, and the layer list is taken
//! exactly as `lookup` built it.
//!
//! ## Cookies
//!
//! A cookie is a position: `.` is `1`, `..` is `2`, and the entry at index `i` is published with
//! cookie `3 + i`. A whiteout keeps its position without being emitted, so a seek always lands on
//! the next unconsumed entry. An entry whose identity must come from the overlay itself carries
//! inode number `0` in the snapshot and is resolved while emitting, through the same lookup that
//! `stat` uses. The two dot names are not entries here: cookies `1` and `2` are reserved for them.

use hashbrown::HashMap;

use super::{OverlayInode, xattr::OverlayXattrType};
use crate::{
    fs::{
        file::InodeType,
        fs_impls::overlayfs::real::read_all_dirents,
        utils::DirentVisitor,
        vfs::path::{Dentry, is_dot_or_dotdot},
    },
    prelude::*,
};

/// One entry of a merged-directory snapshot: its index in the snapshot is its cookie.
pub(super) struct ReaddirEntry {
    name: String,
    type_: InodeType,
    layer_index: usize,
    /// The inode number to publish; `0` means the emit path must resolve it through the overlay.
    overlay_ino: u64,
    is_whiteout: bool,
    /// Whether the winner's identity comes from a lower layer; gates the marker clear.
    is_origin_preserved: bool,
}

impl ReaddirEntry {
    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn type_(&self) -> InodeType {
        self.type_
    }

    /// Returns the inode number to publish, or `0` when the emit path must resolve it.
    pub(super) fn overlay_ino(&self) -> u64 {
        self.overlay_ino
    }

    pub(super) fn is_whiteout(&self) -> bool {
        self.is_whiteout
    }

    /// Creates a candidate from one layer's enumeration answer; the probes fill in both flags.
    fn new(name: &str, type_: InodeType, layer_index: usize, overlay_ino: u64) -> Self {
        Self {
            name: String::from(name),
            type_,
            layer_index,
            overlay_ino,
            is_whiteout: false,
            is_origin_preserved: false,
        }
    }
}

/// One immutable snapshot of a merged directory; its length is fixed where it is built.
pub(super) struct ReaddirCache {
    entries: Box<[ReaddirEntry]>,
}

impl ReaddirCache {
    pub(super) fn entries(&self) -> &[ReaddirEntry] {
        &self.entries
    }
}

pub(super) type OverlayInodeLockPayload = Option<Arc<ReaddirCache>>;

pub(super) type OverlayInodeLockGuard<'a> = MutexGuard<'a, OverlayInodeLockPayload>;

impl OverlayInode {
    /// Returns this directory's current readdir cache.
    ///
    /// The cache is rebuilt under the transaction lock when the slot holds none, and the rebuild
    /// is published into the slot.
    pub(super) fn current_readdir_cache(&self) -> Result<Arc<ReaddirCache>> {
        let mut slot = self.lock();
        if let Some(current) = slot.as_ref() {
            // A snapshot that is still in the slot is current: it is immutable by construction.
            return Ok(current.clone());
        }
        // The rebuild runs under the transaction lock, where every other reader of the slot runs.
        let entries = self.build_merged_entries()?;
        let has_origin_preserved = entries.iter().any(|entry| entry.is_origin_preserved);
        let snapshot = Arc::new(ReaddirCache {
            entries: entries.into_boxed_slice(),
        });
        *slot = Some(snapshot.clone());
        if !has_origin_preserved && let Some(upper_real) = self.upper.get() {
            // The durable marker mirrors this predicate, so it may go once the predicate is false.
            let namespace = self.fs_arc().policy().xattr_namespace();
            let _ = OverlayXattrType::Impure.remove_from(upper_real.dentry(), namespace);
        }
        Ok(snapshot)
    }

    /// Reports whether this directory's merged view holds no visible content.
    ///
    /// The caller's payload is the snapshot the answer is read from, or `None` to rebuild one:
    /// holding the transaction lock across that adoption is what makes the verdict evidence.
    pub(super) fn is_empty_dir(&self, payload: &OverlayInodeLockPayload) -> Result<bool> {
        let rebuilt;
        let entries: &[ReaddirEntry] = match payload {
            Some(snapshot) => snapshot.entries(),
            None => {
                rebuilt = self.build_merged_entries()?;
                &rebuilt
            }
        };
        Ok(!entries.iter().any(|entry| !entry.is_whiteout))
    }

    /// Merges every layer of this directory into one list of entries, lowest layer first.
    fn build_merged_entries(&self) -> Result<Vec<ReaddirEntry>> {
        let fs = self.fs_arc();
        let namespace = fs.policy().xattr_namespace();
        // Without pass-through identity every published number may differ from the layer's own.
        let defer_all = !fs.identity().is_same_fs_passthrough();
        // The impure marker is the durable mirror of "an entry took its identity from a lower".
        let dir_is_impure = match self.upper.get() {
            Some(upper) => OverlayXattrType::Impure.is_positive_on(upper.dentry(), namespace)?,
            None => false,
        };

        let mut entries: Vec<ReaddirEntry> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        // Bottom-up: the lowers run lowest first, and the upper folds in last so that it wins.
        let layers = self.lowers.iter().rev().chain(self.upper.get());
        for layer in layers {
            let layer_index = layer.layer_index();
            let mut layer_entries = read_layer_entries(layer.dentry(), layer_index)?;
            for entry in &mut layer_entries {
                if defer_all || (layer_index == 0 && dir_is_impure) {
                    entry.overlay_ino = 0;
                }
            }
            fold_layer_entries(&mut entries, &mut index, layer_entries);
        }
        drop(index);
        self.resolve_entry_whiteouts(&mut entries)?;
        if dir_is_impure {
            self.resolve_entry_origins(&mut entries)?;
        }
        Ok(entries)
    }

    /// Probes each position's winner for whiteout form, so emit can suppress it in place.
    fn resolve_entry_whiteouts(&self, entries: &mut [ReaddirEntry]) -> Result<()> {
        let fs = self.fs_arc();
        for entry in entries {
            // Only these two forms can carry a whiteout here, so every other type skips the probe.
            if !matches!(entry.type_, InodeType::CharDevice | InodeType::File) {
                continue;
            }
            let layer_dir = self.layer_dir(entry.layer_index)?;
            let child = match layer_dir.as_dir_dentry_or_err()?.lookup_child(&entry.name) {
                Ok(child) => child,
                // A name the layer no longer holds is no whiteout; the merge owns that verdict.
                Err(err) if err.error() == Errno::ENOENT => continue,
                Err(err) => return Err(err),
            };
            entry.is_whiteout = fs.is_whiteout(&child)?;
        }
        Ok(())
    }

    /// Probes each winner for an origin record resolving into this mount, feeding the marker clear.
    fn resolve_entry_origins(&self, entries: &mut [ReaddirEntry]) -> Result<()> {
        let fs = self.fs_arc();
        for entry in entries {
            if entry.layer_index != 0 {
                // A lower winner is its own identity source, so it is origin-preserved already.
                entry.is_origin_preserved = true;
                continue;
            }
            let layer_dir = self.layer_dir(entry.layer_index)?;
            let child = match layer_dir.as_dir_dentry_or_err()?.lookup_child(&entry.name) {
                Ok(child) => child,
                // A name that is already gone carries no record, so it is not origin-preserved.
                Err(err) if err.error() == Errno::ENOENT => continue,
                Err(err) => return Err(err),
            };
            entry.is_origin_preserved = fs.origin_of(&child).is_some();
        }
        Ok(())
    }

    fn layer_dir(&self, layer_index: usize) -> Result<&Arc<Dentry>> {
        let layer_dentry = if layer_index == 0 {
            self.upper.get().map(|upper| upper.dentry())
        } else {
            // A mount-level index is not a stack position: only the materialized lowers are here.
            self.lowers
                .iter()
                .find(|layer| layer.layer_index() == layer_index)
                .map(|lower| lower.dentry())
        };
        layer_dentry.ok_or_else(|| {
            Error::with_message(
                Errno::EIO,
                "the merged entry names a layer this directory does not have",
            )
        })
    }
}

/// Collects one layer's enumeration answer into candidates; collecting itself never fails.
struct LayerEntryVisitor {
    layer_index: usize,
    entries: Vec<ReaddirEntry>,
}

impl DirentVisitor for LayerEntryVisitor {
    fn visit(&mut self, name: &str, ino: u64, type_: InodeType, _offset: usize) -> Result<()> {
        // A vanished name is the merge's business, not an error here: some layers swallow them.
        self.entries
            .push(ReaddirEntry::new(name, type_, self.layer_index, ino));
        Ok(())
    }
}

/// Reads one layer directory to exhaustion into candidates, in that layer's own order.
fn read_layer_entries(layer_dir: &Arc<Dentry>, layer_index: usize) -> Result<Vec<ReaddirEntry>> {
    let dir = layer_dir.as_dir_dentry_or_err()?;
    let mut visitor = LayerEntryVisitor {
        layer_index,
        entries: Vec::new(),
    };
    read_all_dirents(dir.inode(), &mut visitor)?;
    // The two dot names are positions of their own, not entries any layer can contribute.
    let mut entries = visitor.entries;
    entries.retain(|entry| !is_dot_or_dotdot(&entry.name));
    Ok(entries)
}

/// Folds one layer's candidates into the accumulator, applying the merge order.
///
/// A name is answered by the highest layer that holds it.
fn fold_layer_entries(
    entries: &mut Vec<ReaddirEntry>,
    index: &mut HashMap<String, usize>,
    layer: Vec<ReaddirEntry>,
) {
    for source in layer {
        let position = if let Some(position) = index.get(source.name()).copied() {
            position
        } else {
            let position = entries.len();
            // The folding step completes this candidate before any read; `usize::MAX` fails closed.
            entries.push(ReaddirEntry::new(
                source.name(),
                InodeType::Unknown,
                usize::MAX,
                0,
            ));
            index.insert(String::from(source.name()), position);
            position
        };
        entries[position] = source;
    }
}
