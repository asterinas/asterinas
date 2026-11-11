// SPDX-License-Identifier: MPL-2.0

//! The link recipe.
//!
//! The recipe gives the linked name a real upper entry: it classifies the name
//! once, persists the parent's impurity when the source carries an origin
//! record, and then either links the source into place directly or, where an
//! upper whiteout holds the name, takes the name over with a prepared hard-link
//! temp published with [`RenameMode::Replace`], since a real-layer `link` would
//! fail with `EEXIST` there. Temp cleanup on failure is explicit and fallible.
//!
//! A metadata-only source, whose data stays in a lower layer, also receives an
//! absolute redirect record before the link is made.
//!
//! Degradation note: without a persistent origin index (a lower-origin
//! identity map used to deduplicate copy-up targets), two lower aliases of
//! one lower inode that copy up separately may become two distinct upper
//! inodes. This split is accepted: each copy-up is independent, and no
//! origin-index lookup is consulted to share the same upper inode.
//! Upper-authoritative sources already share one upper inode because they
//! are not split by separate copy-up.

use crate::{
    fs::{
        fs_impls::overlayfs::{
            fs::OverlayFs,
            inode::{
                Lookup, NegativeLookup, OverlayInode, OverlayXattrType, redirect::RedirectForm,
            },
        },
        vfs::{
            inode::{Inode, RenameMode},
            path::Dentry,
        },
    },
    prelude::*,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LinkTarget {
    Whiteout,
    Vacant,
}

impl OverlayInode {
    /// Serves as the link entry the VFS calls: it promotes both objects, then composes the link
    /// under the parent lock.
    pub(in super::super) fn link_impl(
        &self,
        self_dentry: &Dentry,
        old_dentry: &Dentry,
        name: &str,
    ) -> Result<()> {
        let fs = self.fs_arc();
        // The receiver and source promote before any lock: the linked name needs a real upper.
        self.writable_meta_object(self_dentry)?;
        let source = self.promote_link_source(old_dentry)?;
        // A metadata-only source needs an absolute redirect record. The source's upper entry
        // now settles whether it holds the data, so this is where the need is known; a mount
        // that cannot create the record refuses the link before the record write.
        let needs_redirect = source.is_metadata_only_regular_file();
        if needs_redirect && !fs.policy().can_create_redirects() {
            return_errno_with_message!(
                Errno::EXDEV,
                "the overlay link of a metadata-only object requires redirect_dir=on"
            );
        }
        let source_upper = source.writable_upper().dentry();
        let _renames_guard = needs_redirect.then(|| fs.lock_renames());
        let mut dir_guard = self.lock();
        let target = self.classify_link_target(&fs, name)?;
        self.set_link_impure_marker(&fs, source_upper)?;
        if needs_redirect {
            source.set_link_redirect(&fs, old_dentry)?;
        }
        self.link_name(&fs, source_upper, name, target)?;
        // The linked name belongs to the parent's next merge: the snapshot lacking it is stale.
        *dir_guard = None;
        Ok(())
    }

    fn promote_link_source(&self, old_dentry: &Dentry) -> Result<Arc<OverlayInode>> {
        let source = Arc::downcast::<OverlayInode>(old_dentry.inode().clone())
            .expect("the link source is not an overlay inode");
        // TODO(VFS gap): the source-side admission (access, regular file, set-id) is the VFS's; its
        // `check_hardlink_source` probes the initial user namespace; overlay keeps no source check.
        source.writable_meta_object(old_dentry)?;
        Ok(source)
    }

    fn classify_link_target(&self, fs: &OverlayFs, name: &str) -> Result<LinkTarget> {
        let target_lookup = fs.lookup(self, name)?;
        if matches!(target_lookup, Lookup::Positive(_)) {
            return_errno!(Errno::ESTALE);
        }
        if matches!(
            target_lookup,
            Lookup::Negative(NegativeLookup::HiddenByWhiteout)
        ) {
            return Ok(LinkTarget::Whiteout);
        }
        Ok(LinkTarget::Vacant)
    }

    fn set_link_impure_marker(&self, fs: &OverlayFs, source_upper: &Arc<Dentry>) -> Result<()> {
        // The source upper is what the source name denotes; its record decides parent impurity.
        let source_is_origin_backed = fs.origin_of(source_upper).is_some();
        // An origin-preserved source makes the parent impure; persist the marker before the link.
        if source_is_origin_backed {
            let upper_parent = self.writable_upper();
            if !fs.policy().can_store_private_xattr() {
                return Err(Error::with_message(
                    Errno::EOPNOTSUPP,
                    "the upper filesystem cannot store the impure marker required for a link",
                ));
            }
            if !OverlayXattrType::Impure
                .is_positive_on(upper_parent.dentry(), fs.policy().xattr_namespace())?
            {
                OverlayXattrType::Impure.set_value_on(
                    upper_parent.dentry(),
                    fs.policy().xattr_namespace(),
                    None,
                )?;
            }
        }
        Ok(())
    }

    fn link_name(
        &self,
        fs: &OverlayFs,
        source_upper: &Arc<Dentry>,
        name: &str,
        target: LinkTarget,
    ) -> Result<()> {
        match target {
            LinkTarget::Whiteout => self.link_over_whiteout(fs, name, source_upper),
            LinkTarget::Vacant => self
                .writable_upper()
                .dentry()
                .as_dir_dentry_or_err()?
                .link(source_upper, name),
        }
    }

    fn link_over_whiteout(&self, fs: &OverlayFs, name: &str, source: &Arc<Dentry>) -> Result<()> {
        let upper_parent = self.writable_upper();
        let upper_workdir = fs.upper_workdir_inuse();
        let temp = upper_workdir.create_workdir_link_temp(name, source)?;
        temp.publish(upper_parent.dentry(), name, RenameMode::Replace)
    }

    /// Returns whether this regular file keeps its data in a lower layer.
    ///
    /// A link of such an object gives its data a second name while both names share one
    /// upper record, so the record must name the object by its path from the overlay root.
    /// The predicate is meaningful after the source's promotion settles whether its upper
    /// entry holds the data.
    fn is_metadata_only_regular_file(&self) -> bool {
        self.type_().is_regular_file() && !self.has_upper_data()
    }

    /// Writes the absolute redirect record a link of this metadata-only file needs.
    ///
    /// The caller has established `is_metadata_only_regular_file` and
    /// `can_create_redirects`; the record is absolute because the two names of one
    /// hard-linked inode share it, so it must not name either of them.
    fn set_link_redirect(&self, fs: &OverlayFs, source_dentry: &Dentry) -> Result<()> {
        debug_assert!(self.is_metadata_only_regular_file());
        let Some(parent_dentry) = source_dentry.parent() else {
            return Ok(());
        };
        let source_name = source_dentry.name();
        let form = RedirectForm::Absolute;
        let Some(redirect) = self.redirect_value_for(fs, &source_name, &parent_dentry, form)?
        else {
            return Ok(());
        };
        let source_upper = self.writable_upper();
        OverlayXattrType::Redirect
            .set_value_on(
                source_upper.dentry(),
                fs.policy().xattr_namespace(),
                Some(redirect.value().as_bytes()),
            )
            .map_err(|_| {
                Error::with_message(
                    Errno::EXDEV,
                    "the overlay failed to write the redirect record required for the link",
                )
            })?;
        Ok(())
    }
}
