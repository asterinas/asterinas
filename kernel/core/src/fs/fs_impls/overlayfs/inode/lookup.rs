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
//! The first non-directory hit terminates the scan. An upper non-directory hit
//! is first probed for a metadata-only record: a malformed record fails the
//! lookup, a valid record on a mount that follows it resolves the data object
//! from the lower layers, and a valid record on a mount that does not fails
//! with `EPERM`.
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

        // The stack moves into the instance; the two probe sides and the data flag are re-read
        // from it here.
        let (upper, mut lowers, data_lower, upper_has_data) = (
            facts.upper,
            facts.lowers,
            facts.data_lower,
            facts.upper_has_data,
        );
        // A separated data source is stored last, which is where the instance's data take point
        // reads it. Every other stack already holds its single lower there.
        if let Some(data_lower) = data_lower {
            debug_assert!(
                !is_directory,
                "a directory's merge stack carries no separated data source"
            );
            lowers.push(data_lower);
            debug_assert_eq!(
                lowers.len(),
                2,
                "a metadata-only lower record has one data object"
            );
        }
        let build = move || {
            let object_id = self.identity().project(real_id, is_directory);
            Arc::new_cyclic(|this| OverlayInode {
                fs: self.self_weak().clone(),
                lowers,
                upper: upper.map_or_else(Once::new, Once::initialized),
                upper_has_data: AtomicBool::new(upper_has_data),
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
                        let metacopy = metacopy::check_metacopy(hit.dentry(), namespace)?;
                        if metacopy.is_none() {
                            let inode = self.project_inode(RealObjectStack::upper_only(hit))?;
                            return Ok(Lookup::Positive(inode));
                        }
                        if !self.policy().is_metacopy_enabled() {
                            return_errno_with_message!(
                                Errno::EPERM,
                                "the mount does not follow a metadata-only copy-up record"
                            );
                        }
                        let Some(data) =
                            self.resolve_metacopy_data(parent, name, &hit, namespace)?
                        else {
                            return_errno_with_message!(
                                Errno::EIO,
                                "the metadata-only copy-up object has no lower data"
                            );
                        };
                        let facts = RealObjectStack::metacopy_upper(hit, vec![data]);
                        let inode = self.project_inode(facts)?;
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
                            let facts =
                                self.lower_hit_stack(parent, lookup_name, hit, namespace)?;
                            let inode = self.project_inode(facts)?;
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

    /// Resolves the data object one metadata-only record names.
    ///
    /// The scan begins at the layer below `record`: a relative value — or none — names the same name
    /// there, and an absolute value re-anchors the walk at that layer's root. `None` means no layer
    /// below the record holds the data.
    fn resolve_metacopy_data(
        &self,
        parent: &OverlayInode,
        lookup_name: &str,
        record: &RealObject,
        namespace: XattrNamespace,
    ) -> Result<Option<RealObject>> {
        let start_layer_index = record.layer_index().saturating_add(1);
        let redirect = redirect::read_redirect(record.dentry(), namespace)?;
        if let Some(absolute_path) = redirect.as_deref().filter(|value| value.starts_with('/')) {
            // At most one object answers a `LowerDataFile` walk: the first regular file that
            // carries no record.
            return Ok(self
                .walk_redirect_path(
                    absolute_path,
                    start_layer_index,
                    RedirectAcceptance::LowerDataFile,
                    namespace,
                )?
                .into_iter()
                .next());
        }
        let lookup_name = redirect.as_deref().unwrap_or(lookup_name);
        parent.first_lower_data_object(lookup_name, start_layer_index, namespace)
    }

    /// Builds the stack one lower non-directory hit publishes: the plain object when it carries no
    /// record, and the record plus its data object otherwise.
    fn lower_hit_stack(
        &self,
        parent: &OverlayInode,
        lookup_name: &str,
        hit: RealObject,
        namespace: XattrNamespace,
    ) -> Result<RealObjectStack> {
        let metacopy = metacopy::check_metacopy(hit.dentry(), namespace)?;
        if metacopy.is_none() {
            return Ok(RealObjectStack::lower_only(hit));
        }
        if !self.policy().is_metacopy_enabled() {
            return_errno_with_message!(
                Errno::EPERM,
                "the mount does not follow a metadata-only copy-up record"
            );
        }
        let Some(data) = self.resolve_metacopy_data(parent, lookup_name, &hit, namespace)? else {
            return_errno_with_message!(
                Errno::EIO,
                "the metadata-only object found in a lower layer has no lower data"
            );
        };
        Ok(RealObjectStack::metacopy_lower(hit, data))
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

    /// Returns the first lower real object holding `name` in the receiver's own lower stack.
    ///
    /// A whiteout counts as a hit that holds nothing, so it answers `None`; the first hit decides,
    /// so the scan never falls through to a deeper lower.
    fn first_lower_object(&self, name: &str) -> Result<Option<RealObject>> {
        let fs = self.fs_arc();
        for lower in &self.lowers {
            // A lower stack element is a directory by construction.
            let lower_dir = lower.dentry().as_dir_dentry_or_err()?;
            match lower_dir.lookup_child(name) {
                Ok(child_dentry) => {
                    let hit = RealObject::new_lower(lower.layer_index(), child_dentry);
                    if fs.is_whiteout(hit.dentry())? {
                        return Ok(None);
                    }
                    return Ok(Some(hit));
                }
                Err(err) if matches!(err.error(), Errno::ENOENT | Errno::ENAMETOOLONG) => continue,
                Err(err) => return Err(err),
            }
        }
        Ok(None)
    }

    /// Returns the first lower regular file under `name` that carries no metadata-only record.
    ///
    /// A whiteout counts as a hit that holds nothing, so it answers `None`, as does a first hit
    /// that is not a regular file; a metadata-only hit carries no data either, so the scan skips it
    /// and continues downward. Only the layers at or below `start_layer_index` are consulted, so a
    /// layer at or above the record is never re-probed.
    fn first_lower_data_object(
        &self,
        name: &str,
        start_layer_index: usize,
        namespace: XattrNamespace,
    ) -> Result<Option<RealObject>> {
        let fs = self.fs_arc();
        for lower in &self.lowers {
            if lower.layer_index() < start_layer_index {
                // A layer at or above the record was already the lookup's to accept.
                continue;
            }
            // A lower stack element is a directory by construction.
            let lower_dir = lower.dentry().as_dir_dentry_or_err()?;
            match lower_dir.lookup_child(name) {
                Ok(child_dentry) => {
                    let hit = RealObject::new_lower(lower.layer_index(), child_dentry);
                    if fs.is_whiteout(hit.dentry())? {
                        return Ok(None);
                    }
                    if !hit.real_inode().type_().is_regular_file() {
                        return Ok(None);
                    }
                    if metacopy::check_metacopy(hit.dentry(), namespace)?.is_some() {
                        // A metadata-only hit carries no data, so the scan continues downward.
                        continue;
                    }
                    return Ok(Some(hit));
                }
                Err(err) if matches!(err.error(), Errno::ENOENT | Errno::ENAMETOOLONG) => continue,
                Err(err) => return Err(err),
            }
        }
        Ok(None)
    }

    /// Returns the type a lower layer physically holds at `name` in the receiver's own lower stack.
    ///
    /// `None` when no lower holds it (a whiteout included), `Some` with the found object's type
    /// when one does, and an error when a lower could not answer. Each caller decides what an
    /// unanswerable probe means for it, and every caller here answers conservatively: a removal
    /// or a rename must not act on an unknown.
    pub(super) fn lower_entry(&self, name: &str) -> Result<Option<InodeType>> {
        Ok(self
            .first_lower_object(name)?
            .map(|hit| hit.real_inode().type_()))
    }
}
