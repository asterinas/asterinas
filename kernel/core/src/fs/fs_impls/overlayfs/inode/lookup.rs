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

use core::sync::atomic::AtomicBool;

use spin::Once;

use crate::{
    fs::{
        file::InodeType,
        fs_impls::overlayfs::{
            fs::OverlayFs,
            inode::{OverlayInode, xattr::OverlayXattrType},
            layer::RealObjectStack,
            real::RealObject,
        },
        vfs::{inode::Extension, path::Dentry, xattr::XattrNamespace},
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

pub(super) fn is_whiteout_inode(real_dentry: &Dentry, namespace: XattrNamespace) -> Result<bool> {
    let metadata = real_dentry.inode().metadata()?;
    if metadata.type_ == InodeType::CharDevice
        && metadata.self_dev_id.is_none_or(|dev_id| dev_id.is_null())
    {
        return Ok(true);
    }
    OverlayXattrType::Whiteout.is_positive_on(real_dentry, namespace)
}

pub(super) fn is_opaque_directory(real_dentry: &Dentry, namespace: XattrNamespace) -> Result<bool> {
    if !real_dentry.inode().type_().is_directory() {
        return Ok(false);
    }
    OverlayXattrType::Opaque.is_positive_on(real_dentry, namespace)
}

impl OverlayFs {
    fn lookup_in_layers(&self, parent: &Arc<OverlayInode>, name: &str) -> Result<Lookup> {
        let namespace = self.policy().xattr_namespace();
        let mut upper_hit: Option<RealObject> = None;
        let mut lower_hits: Vec<RealObject> = Vec::new();

        if let Some(parent_upper) = parent.upper.get() {
            let upper_dir = parent_upper.dentry().as_dir_dentry_or_err()?;
            match upper_dir.lookup_child(name) {
                Ok(child_dentry) => {
                    let hit = RealObject::new_upper(child_dentry);
                    if is_whiteout_inode(hit.dentry(), namespace)? {
                        return Ok(Lookup::Negative(NegativeLookup::HiddenByWhiteout));
                    }
                    if !hit.real_inode().type_().is_directory() {
                        let inode = self.project_inode(RealObjectStack::upper_only(hit))?;
                        return Ok(Lookup::Positive(inode));
                    }
                    if is_opaque_directory(hit.dentry(), namespace)? {
                        let inode = self.project_inode(RealObjectStack::upper_only(hit))?;
                        return Ok(Lookup::Positive(inode));
                    }
                    upper_hit = Some(hit);
                }
                Err(err) if err.error() == Errno::ENOENT => {
                    if is_opaque_directory(parent_upper.dentry(), namespace)? {
                        return Ok(Lookup::Negative(NegativeLookup::Absent));
                    }
                }
                Err(err) => return Err(err),
            }
        }

        for parent_lower in &parent.lowers {
            let layer_index = parent_lower.layer_index();
            let lower_dir = parent_lower.dentry().as_dir_dentry_or_err()?;
            match lower_dir.lookup_child(name) {
                Ok(child_dentry) => {
                    let hit = RealObject::new_lower(layer_index, child_dentry);
                    if is_whiteout_inode(hit.dentry(), namespace)? {
                        // No directory has accumulated below this name yet.
                        if upper_hit.is_none() && lower_hits.is_empty() {
                            return Ok(Lookup::Negative(NegativeLookup::HiddenByWhiteout));
                        }
                        break;
                    }
                    if !hit.real_inode().type_().is_directory() {
                        if upper_hit.is_none() && lower_hits.is_empty() {
                            let inode = self.project_inode(RealObjectStack::lower_only(hit))?;
                            return Ok(Lookup::Positive(inode));
                        }
                        break;
                    }
                    let is_opaque = is_opaque_directory(hit.dentry(), namespace)?;
                    lower_hits.push(hit);
                    if is_opaque {
                        break;
                    }
                }
                Err(err) if err.error() == Errno::ENOENT => continue,
                Err(err) => return Err(err),
            }
        }

        if upper_hit.is_none() && lower_hits.is_empty() {
            return Ok(Lookup::Negative(NegativeLookup::Absent));
        }
        let facts = RealObjectStack::new(upper_hit, lower_hits);
        let inode = self.project_inode(facts)?;
        Ok(Lookup::Positive(inode))
    }

    pub(super) fn lookup(&self, parent: &OverlayInode, name: &str) -> Result<Lookup> {
        let parent = parent.self_arc();
        self.lookup_in_layers(&parent, name)
    }

    /// Projects one real-object stack into the shared overlay inode, reusing a live instance.
    ///
    /// The stack is the only input: identity, published number, then the retained objects move in.
    pub(in crate::fs::fs_impls::overlayfs) fn project_inode(
        &self,
        facts: RealObjectStack,
    ) -> Result<Arc<OverlayInode>> {
        // Step 1 - the real id: source id, type, plus a non-directory upper's record.
        let (real_id, is_directory, is_broken_lower_hard_link) = {
            let visible = facts.visible_source();
            let real = visible.real_inode();
            let is_directory = real.type_().is_directory();
            let identity = self.identity();
            let real_id = if facts.upper.is_none() {
                // A lower-only winner is its own identity: the visible source supplies the pair.
                identity.real_id_of(visible)
            } else if is_directory {
                // A directory never reads a record: its topmost retained lower supplies the pair.
                match facts.lowers.first() {
                    Some(topmost_lower) => identity.real_id_of(topmost_lower),
                    None => identity.real_id_of(visible),
                }
            } else {
                // A non-directory upper is keyed by its durable record, never by the name probe.
                self.origin_of(visible.dentry())
                    .unwrap_or_else(|| identity.real_id_of(visible))
            };
            // A lower non-directory hard link keeps one instance per name; promoting one breaks it.
            let is_broken_lower_hard_link =
                facts.upper.is_none() && !is_directory && real.metadata()?.nr_hard_links > 1;
            (real_id, is_directory, is_broken_lower_hard_link)
        };

        // Step 2 - the published number, drawn once here and repeated for the instance's lifetime.
        if !is_broken_lower_hard_link
            && let Some(occupant) =
                self.inodes()
                    .reusable_occupant(real_id, facts.upper.as_ref(), facts.lowers.first())
        {
            return Ok(occupant);
        }

        // The stack moves into the instance; the two probe sides are re-read from it below.
        let (upper, lowers) = (facts.upper, facts.lowers);
        let build = move || {
            let object_id = self.identity().project(real_id, is_directory);
            Arc::new_cyclic(|this| OverlayInode {
                fs: self.self_weak().clone(),
                lowers,
                upper: upper.map_or_else(Once::new, Once::initialized),
                name_taken: AtomicBool::new(false),
                object_id,
                lock: Mutex::new(None),
                extension: Extension::new(),
                this: this.clone(),
            })
        };
        if is_broken_lower_hard_link {
            return Ok(build());
        }
        let inode = build();
        Ok(self.inodes().register(real_id, inode))
    }
}

impl OverlayInode {
    /// Whether this instance still projects the same real objects as the probe's two sides.
    ///
    /// Each side the probe has must match this instance's; it is the cache entry's anchor check.
    pub(super) fn is_reusable_for(
        &self,
        upper: Option<&RealObject>,
        topmost_lower: Option<&RealObject>,
    ) -> bool {
        if let Some(probe_upper) = upper
            && !self
                .upper
                .get()
                .is_some_and(|upper| Arc::ptr_eq(upper.real_inode(), probe_upper.real_inode()))
        {
            return false;
        }
        if let Some(probe_lower) = topmost_lower
            && !self
                .lowers
                .first()
                .is_some_and(|lower| Arc::ptr_eq(lower.real_inode(), probe_lower.real_inode()))
        {
            return false;
        }
        true
    }

    /// Whether any lower layer physically holds `name`; a whiteout is a miss, an error a hit.
    pub(super) fn has_lower_entry(&self, name: &str, namespace: XattrNamespace) -> Result<bool> {
        for lower in &self.lowers {
            // A lower stack element is a directory by construction; a failure is an unknown.
            let Ok(lower_dir) = lower.dentry().as_dir_dentry_or_err() else {
                return Ok(true);
            };
            match lower_dir.lookup_child(name) {
                Ok(child_dentry) => {
                    let hit = RealObject::new_lower(lower.layer_index(), child_dentry);
                    return match is_whiteout_inode(hit.dentry(), namespace) {
                        Ok(is_whiteout) => Ok(!is_whiteout),
                        Err(_) => Ok(true),
                    };
                }
                Err(err) if matches!(err.error(), Errno::ENOENT | Errno::ENAMETOOLONG) => continue,
                Err(_) => return Ok(true),
            }
        }
        Ok(false)
    }
}
