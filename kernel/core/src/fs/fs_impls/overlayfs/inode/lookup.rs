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

#![short_vis_path::add(overlayfs)]

use core::sync::atomic::AtomicBool;

use spin::Once;

use crate::{
    fs::{
        file::InodeType,
        fs_impls::overlayfs::{
            fs::OverlayFs,
            inode::{OverlayInode, xattr::OverlayXattrType},
            real::{RealObject, RealObjectStack},
        },
        vfs::{inode::Extension, path::Dentry},
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

impl OverlayFs {
    pub(super) fn lookup(&self, parent: &OverlayInode, name: &str) -> Result<Lookup> {
        self.lookup_in_layers(parent, name)
    }

    /// Projects one real-object stack into the shared overlay inode, reusing a live instance.
    ///
    /// The stack is the only input: identity, published number, then the retained objects move in.
    pub(in overlayfs) fn project_inode(&self, facts: RealObjectStack) -> Result<Arc<OverlayInode>> {
        // Step 1 - the real id: the identity module resolves it from the stack, record included.
        let visible = facts.visible_source();
        let is_directory = visible.real_inode().type_().is_directory();
        let real_id = self.real_id_of_stack(&facts);
        // A lower non-directory hard link keeps one instance per name; promoting one breaks it.
        let is_broken_lower_hard_link = facts.upper.is_none()
            && !is_directory
            && visible.real_inode().metadata()?.nr_hard_links > 1;

        // Step 2 - the published number, drawn once here and repeated for the instance's lifetime.
        if !is_broken_lower_hard_link && let Some(occupant) = self.inodes().occupant(real_id) {
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

    fn lookup_in_layers(&self, parent: &OverlayInode, name: &str) -> Result<Lookup> {
        let namespace = self.policy().xattr_namespace();
        let mut upper_hit: Option<RealObject> = None;
        let mut lower_hits: Vec<RealObject> = Vec::new();

        if let Some(parent_upper) = parent.upper.get() {
            let upper_dir = parent_upper.dentry().as_dir_dentry_or_err()?;
            match upper_dir.lookup_child(name) {
                Ok(child_dentry) => {
                    let hit = RealObject::new_upper(child_dentry);
                    if self.is_whiteout(hit.dentry())? {
                        return Ok(Lookup::Negative(NegativeLookup::HiddenByWhiteout));
                    }
                    if !hit.real_inode().type_().is_directory() {
                        let inode = self.project_inode(RealObjectStack::upper_only(hit))?;
                        return Ok(Lookup::Positive(inode));
                    }
                    if OverlayXattrType::Opaque.is_positive_on(hit.dentry(), namespace)? {
                        let inode = self.project_inode(RealObjectStack::upper_only(hit))?;
                        return Ok(Lookup::Positive(inode));
                    }
                    upper_hit = Some(hit);
                }
                Err(err) if err.error() == Errno::ENOENT => {
                    if OverlayXattrType::Opaque.is_positive_on(parent_upper.dentry(), namespace)? {
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
                    if self.is_whiteout(hit.dentry())? {
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
                    let is_opaque =
                        OverlayXattrType::Opaque.is_positive_on(hit.dentry(), namespace)?;
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
}

impl OverlayInode {
    /// Whether the name's upper entry disappeared behind the caller's back.
    ///
    /// The caller's dentry says the name has an upper object, while this fresh projection of the same
    /// name has none.
    pub(super) fn is_stale_upper_target(&self, child_dentry: &Dentry) -> bool {
        let caller_has_upper = child_dentry
            .inode()
            .downcast_ref::<OverlayInode>()
            .is_some_and(|caller| caller.upper.get().is_some());
        self.upper.get().is_none() && caller_has_upper
    }

    /// The type a lower layer physically holds at `name` in the receiver's own lower stack: `None`
    /// when no lower holds it (a whiteout included), `Some` with the found object's type when one
    /// does, and an error when a lower could not answer.
    ///
    /// Each caller decides what an unanswerable probe means for it, and every caller here answers
    /// conservatively: a removal or a rename must not act on an unknown.
    pub(super) fn lower_entry(&self, name: &str) -> Result<Option<InodeType>> {
        let fs = self.fs_arc();
        for lower in &self.lowers {
            // A lower stack element is a directory by construction.
            let lower_dir = lower.dentry().as_dir_dentry_or_err()?;
            match lower_dir.lookup_child(name) {
                Ok(child_dentry) => {
                    let hit = RealObject::new_lower(lower.layer_index(), child_dentry);
                    return match fs.is_whiteout(hit.dentry())? {
                        // A whiteout hides the name rather than holding it.
                        true => Ok(None),
                        false => Ok(Some(hit.real_inode().type_())),
                    };
                }
                Err(err) if matches!(err.error(), Errno::ENOENT | Errno::ENAMETOOLONG) => continue,
                Err(err) => return Err(err),
            }
        }
        Ok(None)
    }
}
