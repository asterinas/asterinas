// SPDX-License-Identifier: MPL-2.0

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
//! The **anchor path** is the layer-relative path of the visible-source
//! real dentry, collected by walking `Dentry::parent` up to the layer root
//! (pointer comparison — the clone view re-roots the same dentry `Arc`s, so
//! the chain does not stop on a parentless dentry). The readdir `..` entry
//! is its only consumer: re-walking `lookup` from the mount root over that
//! path re-resolves the current parent overlay object; a failed walk is
//! anchor-path divergence and the consumer degrades, never publishes blind.

#![short_vis_path::add(overlayfs)]

use spin::Once;

use crate::{
    fs::{
        file::InodeType,
        fs_impls::overlayfs::{
            fs::OverlayFs,
            inode::{
                OverlayInode, ReaddirCache,
                xattr::{MarkerReadSemantics, OverlayRecordName, has_marker},
            },
            layer::RealObjectStack,
            real::RealObject,
        },
        vfs::{
            file_system::FileSystem,
            inode::{Extension, Inode},
            xattr::XattrNamespace,
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
    namespace: XattrNamespace,
) -> Result<bool> {
    let metadata = real_inode.metadata()?;
    if metadata.type_ == InodeType::CharDevice
        && metadata.self_dev_id.is_none_or(|dev_id| dev_id.is_null())
    {
        return Ok(true);
    }
    has_marker(
        real_inode,
        OverlayRecordName::Whiteout.construct_xattr_name(namespace)?,
        MarkerReadSemantics::ValueY,
    )
}

pub(super) fn is_opaque_directory(real: &RealObject, namespace: XattrNamespace) -> Result<bool> {
    if !real.real_inode().type_().is_directory() {
        return Ok(false);
    }
    has_marker(
        real.real_inode(),
        OverlayRecordName::Opaque.construct_xattr_name(namespace)?,
        MarkerReadSemantics::ValueY,
    )
}

impl OverlayFs {
    fn lookup_in_layers(&self, parent: &Arc<OverlayInode>, name: &str) -> Result<Lookup> {
        let namespace = self.policy().xattr_namespace();
        let mut dir_hits: Vec<RealObject> = Vec::new();

        if let Some(upper_real) = parent.upper.get() {
            let upper_dir = upper_real.dentry().as_dir_dentry_or_err()?;
            match upper_dir.lookup_child(name) {
                Ok(child_dentry) => {
                    let hit = RealObject::new(0, child_dentry);
                    if is_whiteout_inode(hit.real_inode(), namespace)? {
                        return Ok(Lookup::Negative(NegativeLookup::HiddenByWhiteout));
                    }
                    if !hit.real_inode().type_().is_directory() {
                        return Ok(Lookup::Positive(
                            self.project_inode(&RealObjectStack::upper_only(hit)),
                        ));
                    }
                    if is_opaque_directory(&hit, namespace)? {
                        return Ok(Lookup::Positive(
                            self.project_inode(&RealObjectStack::upper_only(hit)),
                        ));
                    }
                    dir_hits.push(hit);
                }
                Err(err) if err.error() == Errno::ENOENT => {
                    if is_opaque_directory(upper_real, namespace)? {
                        return Ok(Lookup::Negative(NegativeLookup::Absent));
                    }
                }
                Err(err) => return Err(err),
            }
        }

        for lower_real in &parent.lowers {
            let layer_index = lower_real.layer_index();
            let lower_dir = lower_real.dentry().as_dir_dentry_or_err()?;
            match lower_dir.lookup_child(name) {
                Ok(child_dentry) => {
                    let hit = RealObject::new(layer_index, child_dentry);
                    if is_whiteout_inode(hit.real_inode(), namespace)? {
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
                    let is_opaque = is_opaque_directory(&hit, namespace)?;
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
        let parent = parent.self_arc()?;
        self.lookup_in_layers(&parent, name)
    }

    /// A valid cache hit skips the origin read; a miss reads it before taking the write guard.
    pub(in overlayfs) fn project_inode(&self, facts: &RealObjectStack) -> Arc<OverlayInode> {
        let source = facts.visible_source();
        let key = self.real_object_key(source);
        let is_directory = facts.is_merged() || source.real_inode().type_().is_directory();
        let source_inode = facts.visible_source().real_inode().clone();
        // A cache hit is valid only when its visible source is the same real object.
        if let Some(inode) = self.inodes().get(key)
            && Arc::ptr_eq(inode.visible_source().real_inode(), &source_inode)
        {
            return inode;
        }
        let layer = self.layer(source.layer_index());
        let fallback_fn = || {
            self.identity().project(
                layer.fsid,
                source.real_inode().ino(),
                layer.container_dev_id,
                is_directory,
            )
        };
        let object_id = if source.layer_index() == 0 {
            match self.project_origin_object_id(source.real_inode(), &facts.lowers, is_directory) {
                Ok(Some(object_id)) => object_id,
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
            // Reusable only when it points at this same real object, not merely the same lower.
            move |carrier| Arc::ptr_eq(carrier.visible_source().real_inode(), &source_inode),
            move || {
                let upper = match upper {
                    Some(upper) => Once::initialized(upper),
                    None => Once::new(),
                };
                let lock = Mutex::new(if is_directory {
                    Some(ReaddirCache::new())
                } else {
                    None
                });
                Arc::new_cyclic(|this| OverlayInode {
                    fs,
                    lowers,
                    upper,
                    object_id,
                    lock,
                    copyup: Mutex::new(()),
                    extension: Extension::new(),
                    this: this.clone(),
                })
            },
        )
    }
}

impl OverlayInode {
    /// Layer-relative anchor path of the visible source, used only for the readdir `..` entry.
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
    /// Re-resolves the current overlay object at an anchor path by walking `lookup` from the root.
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
