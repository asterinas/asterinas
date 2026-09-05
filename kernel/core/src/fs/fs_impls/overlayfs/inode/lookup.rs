// SPDX-License-Identifier: MPL-2.0
#![short_vis_path::add(overlayfs)]

//! Upper-first layer lookup and inode projection.
//!
//! This module owns the lookup path: it resolves a name upper-first across
//! the layer stack, projects the winning real-object stack into the shared
//! [`OverlayInode`], and returns the simple positive/negative [`Lookup`]
//! result.
//!
//! # Lookup scan
//!
//! The lookup scan is upper-first with overlayfs merge-stop semantics.
//! The first non-directory hit terminates as a single-object result.
//! Directory hits accumulate into the lower stack until a barrier stops
//! the scan: a whiteout, an opaque directory, or a non-directory below
//! an accumulated directory. An opaque upper parent also stops the scan
//! before the lowers; it leaves the absent upper name invisible in the
//! whole overlay.
//!
//! # Anchor re-resolution
//!
//! A dentry-less entry derives its namespace position from its **anchor
//! path**: the layer-relative path of the visible-source real dentry,
//! collected by walking `Dentry::parent` up to the layer root (pointer
//! comparison — the clone view re-roots the same dentry `Arc`s, so the
//! chain does not stop on a parentless dentry). Re-walking `lookup` from
//! the mount root over that path re-resolves the current overlay objects;
//! a failed walk is anchor-path divergence and every consumer fails closed
//! or degrades, never publishes blind.

use spin::Once;

use crate::{
    fs::{
        file::InodeType,
        fs_impls::overlayfs::{
            fs::OverlayFs,
            inode::{
                OverlayInode, ReaddirIndex,
                xattr::{
                    MarkerReadSemantics, OverlayRecordName, OverlayXattrPrefix, has_marker,
                    overlay_record_name,
                },
            },
            layer::RealObjectStack,
            real::RealObject,
        },
        vfs::{
            file_system::FileSystem,
            inode::{Extension, Inode},
        },
    },
    prelude::*,
};

#[derive(Clone)]
pub(super) enum Lookup {
    Positive(Arc<OverlayInode>),
    Negative(NegativeLookup),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NegativeLookup {
    Absent,
    HiddenByWhiteout,
}

pub(super) fn is_whiteout_inode(
    real_inode: &Arc<dyn Inode>,
    prefix: OverlayXattrPrefix,
) -> Result<bool> {
    let metadata = real_inode.metadata()?;
    if metadata.type_ == InodeType::CharDevice
        && metadata.self_dev_id.is_none_or(|dev_id| dev_id.is_null())
    {
        return Ok(true);
    }
    has_marker(
        real_inode,
        overlay_record_name(OverlayRecordName::Whiteout, prefix)?,
        MarkerReadSemantics::ValueY,
    )
}

pub(super) fn is_opaque_directory(real: &RealObject, prefix: OverlayXattrPrefix) -> Result<bool> {
    if !real.real_inode().type_().is_directory() {
        return Ok(false);
    }
    has_marker(
        real.real_inode(),
        overlay_record_name(OverlayRecordName::Opaque, prefix)?,
        MarkerReadSemantics::ValueY,
    )
}

impl OverlayFs {
    fn lookup_in_layers(&self, parent: &Arc<OverlayInode>, name: &str) -> Result<Lookup> {
        let prefix = self.policy().xattr_prefix();
        let mut dir_hits: Vec<RealObject> = Vec::new();

        if let Some(upper_real) = parent.upper.get() {
            let upper_path = self.real_object_path(upper_real);
            match crate::fs::fs_impls::overlayfs::lookup_child_path(&upper_path, name) {
                Ok(child_path) => {
                    let hit = RealObject::new(0, child_path.dentry().clone());
                    if is_whiteout_inode(hit.real_inode(), prefix)? {
                        return Ok(Lookup::Negative(NegativeLookup::HiddenByWhiteout));
                    }
                    if !hit.real_inode().type_().is_directory() {
                        return Ok(Lookup::Positive(
                            self.project_inode(&RealObjectStack::upper_only(hit)),
                        ));
                    }
                    if is_opaque_directory(&hit, prefix)? {
                        return Ok(Lookup::Positive(
                            self.project_inode(&RealObjectStack::upper_only(hit)),
                        ));
                    }
                    dir_hits.push(hit);
                }
                Err(err) if err.error() == Errno::ENOENT => {
                    if is_opaque_directory(upper_real, prefix)? {
                        return Ok(Lookup::Negative(NegativeLookup::Absent));
                    }
                }
                Err(err) => return Err(err),
            }
        }

        for lower_real in &parent.lowers {
            let layer_index = lower_real.layer_index();
            let lower_path = self.real_object_path(lower_real);
            match crate::fs::fs_impls::overlayfs::lookup_child_path(&lower_path, name) {
                Ok(child_path) => {
                    let hit = RealObject::new(layer_index, child_path.dentry().clone());
                    if is_whiteout_inode(hit.real_inode(), prefix)? {
                        if dir_hits.is_empty() {
                            return Ok(Lookup::Negative(NegativeLookup::HiddenByWhiteout));
                        }
                        break;
                    }
                    if !hit.real_inode().type_().is_directory() {
                        if dir_hits.is_empty() {
                            return Ok(Lookup::Positive(
                                self.project_inode(&RealObjectStack::lower_only(hit)),
                            ));
                        }
                        break;
                    }
                    let is_opaque = is_opaque_directory(&hit, prefix)?;
                    dir_hits.push(hit);
                    if is_opaque {
                        break;
                    }
                }
                Err(err) if err.error() == Errno::ENOENT => continue,
                Err(err) => return Err(err),
            }
        }

        if dir_hits.is_empty() {
            return Ok(Lookup::Negative(NegativeLookup::Absent));
        }

        let upper = if dir_hits[0].layer_index() == 0 {
            Some(dir_hits.remove(0))
        } else {
            None
        };
        Ok(Lookup::Positive(
            self.project_inode(&RealObjectStack::new(upper, dir_hits)),
        ))
    }

    pub(super) fn lookup(&self, parent: &OverlayInode, name: &str) -> Result<Lookup> {
        let parent_arc = self.inodes().get(parent.key(self)).ok_or_else(|| {
            Error::with_message(
                Errno::EIO,
                "the overlay parent is not registered under its visible-source key",
            )
        })?;
        self.lookup_in_layers(&parent_arc, name)
    }

    /// A valid cache hit returns without the lower-id origin read; on a miss
    /// that read completes before the inode-cache write guard is taken.
    pub(super) fn project_inode(&self, facts: &RealObjectStack) -> Arc<OverlayInode> {
        let source = facts.visible_source();
        let key = self.real_object_key(source);
        let is_directory = facts.is_merged() || source.real_inode().type_().is_directory();
        let source_inode = facts.visible_source().real_inode().clone();
        let fresh_is_lower_only = facts.upper.is_none();
        if let Some(inode) = self.inodes().get(key) {
            let hit_valid = if fresh_is_lower_only {
                // A stale-upper inode must not be reused even though
                // `contains_real_inode` matches the retained lower: its
                // visible source is an upper object that no longer
                // resolves, so the metadata it reports would be wrong.
                Arc::ptr_eq(inode.visible_source().real_inode(), &source_inode)
            } else {
                inode.contains_real_inode(&source_inode)
            };
            if hit_valid {
                return inode;
            }
        }
        let layer = self.layer(source.layer_index());
        let fallback_fn = || {
            self.identity()
                .project_object_id(layer, source, is_directory)
        };
        let object_id = if source.layer_index() == 0 {
            match self.read_lower_id(source.real_inode()) {
                Ok(Some(record)) => {
                    if self.origin_real_ino_resolves(&record, facts) {
                        self.identity()
                            .project_object_id_from_lower_id(&record, is_directory)
                            .unwrap_or_else(fallback_fn)
                    } else {
                        fallback_fn()
                    }
                }
                Ok(None) => fallback_fn(),
                Err(err) => {
                    warn!(
                        "failed to read the lower-id record of the upper source; \
                         falling back to the visible-source projection: {:?}",
                        err
                    );
                    fallback_fn()
                }
            }
        } else {
            fallback_fn()
        };
        let fs = self.self_weak().clone();
        let lowers = facts.lowers.clone();
        let upper = facts.upper.clone();
        self.inodes().get_or_create(
            key,
            move |carrier| {
                if fresh_is_lower_only {
                    // A stale-upper inode must not be reused even though
                    // `contains_real_inode` matches the retained lower: its
                    // visible source is an upper object that no longer
                    // resolves, so the metadata it reports would be wrong.
                    Arc::ptr_eq(carrier.visible_source().real_inode(), &source_inode)
                } else {
                    carrier.contains_real_inode(&source_inode)
                }
            },
            move || {
                let upper = match upper {
                    Some(upper) => Once::initialized(upper),
                    None => Once::new(),
                };
                let lock = Mutex::new(if is_directory {
                    Some(ReaddirIndex::new())
                } else {
                    None
                });
                Arc::new(OverlayInode {
                    fs,
                    lowers,
                    upper,
                    object_id,
                    lock,
                    copyup: Mutex::new(()),
                    extension: Extension::new(),
                })
            },
        )
    }
}

impl OverlayInode {
    /// Layer-relative anchor path of the visible source, for dentry-less
    /// entries (the `CopyUpOrigin::Anchor` call sites, e.g. `fallocate`);
    /// fails closed (EIO) if the chain escapes the layer root.
    ///
    /// The clone-view layer re-roots the same dentry `Arc`s, so the walk
    /// terminates by pointer comparison against the layer root, not by the
    /// absence of a parent.
    pub(super) fn anchor_path(&self, fs: &OverlayFs) -> Result<Vec<String>> {
        let source = self.visible_source();
        let layer_root = fs.layer(source.layer_index()).root_dentry();
        let mut components = Vec::new();
        let mut current = source.dentry().clone();
        loop {
            if Arc::ptr_eq(&current, layer_root) {
                components.reverse();
                return Ok(components);
            }
            let Some(parent) = current.parent() else {
                return Err(Error::with_message(
                    Errno::EIO,
                    "the visible-source dentry chain does not reach its layer root",
                ));
            };
            components.push(current.name());
            current = parent;
        }
    }
}

impl OverlayFs {
    /// Re-resolves the CURRENT overlay object at an anchor path by walking
    /// `lookup` from the mount root (barrier semantics reused, not
    /// reimplemented). A negative lookup surfaces as `ENOENT`; consumers
    /// treat any `Err` as anchor-path divergence and fail closed or
    /// degrade, never publish blind.
    pub(super) fn resolve_at_anchor(&self, anchor: &[String]) -> Result<Arc<OverlayInode>> {
        let root = self.root_inode();
        let mut current = Arc::downcast::<OverlayInode>(root).map_err(|_| {
            Error::with_message(Errno::EIO, "the overlay mount root is not an overlay inode")
        })?;
        for name in anchor {
            match self.lookup(&current, name)? {
                Lookup::Positive(inode) => current = inode,
                Lookup::Negative(_) => {
                    return Err(Error::with_message(
                        Errno::ENOENT,
                        "the anchor path no longer resolves in the overlay",
                    ));
                }
            }
        }
        Ok(current)
    }
}
