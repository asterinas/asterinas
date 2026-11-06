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
//! A merged directory hit may carry a redirect record: a relative value
//! renames the name the deeper layers are scanned for, and an absolute
//! value re-anchors that scan at one lower layer's root.

#![short_vis_path::add(overlayfs)]

use core::sync::atomic::AtomicBool;

use spin::Once;

use crate::{
    fs::{
        file::InodeType,
        fs_impls::overlayfs::{
            fs::OverlayFs,
            inode::{OverlayInode, copyup::metacopy, redirect, xattr::OverlayXattrType},
            real::{RealObject, RealObjectStack},
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

/// The largest number of redirect rewrites one walk may perform before it fails with `ELOOP`.
const REDIRECT_MAX_RESTARTS: usize = 8;

/// Selects what one redirect-following walk accepts from the layers it consults.
#[derive(Clone, Copy)]
enum RedirectAcceptance {
    /// Accepts each layer's landing directory, topmost layer first, until a barrier stops the walk,
    /// and keeps every directory an absolute rewrite displaces.
    LayerDirectories,
    /// Accepts the first regular file with no metadata-only record, and ends the walk there.
    LowerDataFile,
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
        let is_follow_enabled = self.policy().can_follow_redirects();
        let mut upper_hit: Option<RealObject> = None;
        let mut lower_hits: Vec<RealObject> = Vec::new();
        let mut redirected_name: Option<String> = None;

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
                    if is_follow_enabled {
                        match redirect::read_redirect(hit.dentry(), namespace)? {
                            Some(value) if value.starts_with('/') => {
                                let lower_hits = self.walk_redirect_path(
                                    &value,
                                    1,
                                    RedirectAcceptance::LayerDirectories,
                                    namespace,
                                )?;
                                let facts = RealObjectStack::new(Some(hit), lower_hits);
                                let inode = self.project_inode(facts)?;
                                return Ok(Lookup::Positive(inode));
                            }
                            Some(value) => redirected_name = Some(value),
                            None => {}
                        }
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
            let lookup_name = redirected_name.as_deref().unwrap_or(name);
            match lower_dir.lookup_child(lookup_name) {
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
                    let redirect = if is_follow_enabled && !is_opaque {
                        redirect::read_redirect(hit.dentry(), namespace)?
                    } else {
                        None
                    };
                    lower_hits.push(hit);
                    if is_opaque {
                        break;
                    }
                    match redirect {
                        Some(value) if value.starts_with('/') => {
                            lower_hits.extend(self.walk_redirect_path(
                                &value,
                                layer_index + 1,
                                RedirectAcceptance::LayerDirectories,
                                namespace,
                            )?);
                            break;
                        }
                        Some(value) => redirected_name = Some(value),
                        None => {}
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

    /// Resolves one absolute redirect path downward from `start_layer_index` under `acceptance`.
    ///
    /// A directory component's redirect is read for every component: a relative value replaces that
    /// component at the same parent, an absolute value re-anchors the remaining path at the layer
    /// root. Under `LayerDirectories`, the directory an absolute rewrite displaces is accepted into
    /// the result before the rewrite, so a carrier keeps contributing its children.
    /// `REDIRECT_MAX_RESTARTS` bounds the rewrites; exceeding it is `ELOOP`.
    ///
    /// `start_layer_index` is a 1-based mount layer index, matching `RealObject::layer_index`; the
    /// walk visits that layer and every deeper one. A layer's walk ends by landing on a directory,
    /// by a miss, or by a barrier (a whiteout, an opaque directory, or a non-directory); a miss and
    /// a metadata-only skip leave the next layer to be consulted, a barrier ends the walk, and an
    /// absolute rewrite clears the barrier. Both the input and every value the walk follows have
    /// passed `decode_redirect`, so each rewrite contributes at least one component.
    fn walk_redirect_path(
        &self,
        absolute_path: &str,
        start_layer_index: usize,
        acceptance: RedirectAcceptance,
        namespace: XattrNamespace,
    ) -> Result<Vec<RealObject>> {
        let mut path: Vec<String> = absolute_path
            .split('/')
            .filter(|component| !component.is_empty())
            .map(String::from)
            .collect();
        // The codec rejects a path without a component, so this only guards the split above.
        if path.is_empty() {
            return Ok(Vec::new());
        }
        debug_assert!(start_layer_index >= 1);

        let lower_layers = self.lower_layers();
        let mut accepted: Vec<RealObject> = Vec::new();
        let mut layer_index = start_layer_index;
        let mut is_barred = false;
        let mut restarts = 0;
        while layer_index <= lower_layers.len() {
            let mut current = lower_layers[layer_index - 1].root_dentry().clone();
            let mut position = 0;
            loop {
                let is_final = position + 1 == path.len();
                let current_dir = current.as_dir_dentry_or_err()?;
                let child = match current_dir.lookup_child(&path[position]) {
                    Ok(child) => child,
                    Err(err) if matches!(err.error(), Errno::ENOENT | Errno::ENAMETOOLONG) => {
                        // A miss leaves this layer without a landing; the next layer may still
                        // hold the path.
                        break;
                    }
                    Err(err) => return Err(err),
                };
                if self.is_whiteout(&child)? {
                    // A whiteout hides the name here and in every deeper layer.
                    is_barred = true;
                    break;
                }
                if !child.inode().type_().is_directory() {
                    if !is_final {
                        // A non-directory cannot be descended into.
                        is_barred = true;
                        break;
                    }
                    if !matches!(acceptance, RedirectAcceptance::LowerDataFile) {
                        // `LayerDirectories` accepts only a directory.
                        is_barred = true;
                        break;
                    }
                    if !child.inode().type_().is_regular_file() {
                        // Only a regular file can hold the data; any other type bars the deeper
                        // layers.
                        is_barred = true;
                        break;
                    }
                    if metacopy::check_metacopy(&child, namespace)?.is_some() {
                        // A metadata-only hit carries no data, so the walk continues deeper.
                        break;
                    }
                    return Ok(vec![RealObject::new_lower(layer_index, child)]);
                }
                if is_final && matches!(acceptance, RedirectAcceptance::LowerDataFile) {
                    // A metadata-only copy-up needs a regular file, never a directory.
                    is_barred = true;
                    break;
                }
                let is_opaque = OverlayXattrType::Opaque.is_positive_on(&child, namespace)?;
                if is_opaque {
                    // An opaque segment still resolves here and bars the deeper layers, unless its
                    // own absolute redirect re-anchors the walk below.
                    is_barred = true;
                }
                if let Some(value) = redirect::read_redirect(&child, namespace)? {
                    if value.starts_with('/') {
                        if restarts == REDIRECT_MAX_RESTARTS {
                            return_errno_with_message!(
                                Errno::ELOOP,
                                "the redirect chain exceeds the restart budget"
                            );
                        }
                        restarts += 1;
                        // The displaced directory keeps feeding the merged view of this name.
                        if matches!(acceptance, RedirectAcceptance::LayerDirectories) {
                            accepted.push(RealObject::new_lower(layer_index, child));
                        }
                        let mut rewritten: Vec<String> = value
                            .split('/')
                            .filter(|component| !component.is_empty())
                            .map(String::from)
                            .collect();
                        rewritten.append(&mut path.split_off(position + 1));
                        path = rewritten;
                        current = lower_layers[layer_index - 1].root_dentry().clone();
                        position = 0;
                        // An absolute redirect below forces the deeper layers back in.
                        is_barred = false;
                        continue;
                    }
                    if value != path[position] {
                        if restarts == REDIRECT_MAX_RESTARTS {
                            return_errno_with_message!(
                                Errno::ELOOP,
                                "the redirect chain exceeds the restart budget"
                            );
                        }
                        restarts += 1;
                        // The name is replaced in place: same layer, same parent, no carrier.
                        path[position] = value;
                        continue;
                    }
                    // A relative value naming the component it replaces is not a rewrite.
                }
                if is_final {
                    // Only `LayerDirectories` reaches a final directory.
                    accepted.push(RealObject::new_lower(layer_index, child));
                    break;
                }
                current = child;
                position += 1;
            }
            if is_barred {
                break;
            }
            layer_index += 1;
        }
        Ok(accepted)
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
