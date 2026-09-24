// SPDX-License-Identifier: MPL-2.0

//! The rename recipe: the one namespace mutation this module owns, entered only
//! through [`OverlayInode::rename_impl`].
//!
//! A rename arrives as a request on the merged view, while every name it changes
//! lives in the upper alone. The source name may still be held by a lower layer
//! after the source leaves it, and the target name is one of three things:
//! nothing the merged view can see, a name an upper whiteout keeps negative, or a
//! visible object that this move overwrites or exchanges.
//!
//! Everything the move touches has to exist in the upper before the real rename
//! can express it, so the objects involved are promoted first and the rename then
//! runs on the upper alone. The upper has to hold the state the merged view
//! expects afterwards: an upper whiteout at the old name wherever a lower layer
//! would otherwise keep that name alive, a whiteout carried away from the target
//! name by the exchange that takes a name there, and an opaque record on a moved
//! directory whose new name meets a lower directory.

use crate::{
    fs::{
        fs_impls::overlayfs::{
            fs::mount::policy::MountPolicy,
            inode::{
                Lookup, NegativeLookup, OverlayInode, OverlayInodeLockGuard, OverlayXattrType,
            },
        },
        vfs::{
            inode::{Inode, RenameMode},
            path::Dentry,
        },
    },
    prelude::*,
};

/// Records how the old name's visibility barrier is settled by the move, and what becomes of a
/// whiteout standing at the target name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WhiteoutAction {
    /// Sets no new barrier: no lower entry hides there, or the exchange keeps the name visible.
    None,
    /// Creates a fresh whiteout at the old name, hiding the lower entry the source leaves behind.
    Create,
    /// Exchanges the target's whiteout onto the old name, where it serves as the barrier.
    Exchange,
    /// Exchanges the target's whiteout onto the old name and deletes it: nothing hides below.
    ExchangeThenDelete,
}

/// Classifies the target name once for one move: the merged view's answer for that name
/// crossed with what the upper physically holds there.
enum RenameTarget {
    /// Applies where the name has no upper entry, a lower whiteout hiding it landing here too.
    Absent,
    /// Applies where an upper whiteout alone holds the name, invisible yet physically taken.
    UpperWhiteout,
    /// Applies where the name resolves in the merged view, to a directory or to anything else.
    Visible {
        /// Holds the object the name resolves to, which a move that overwrites this name displaces.
        inode: Arc<OverlayInode>,
    },
}

/// Holds one rename's whole input, settled in a single pass before the lock set is taken.
struct RenameFacts<'x> {
    /// Reports whether a lower layer holds the new name as a directory.
    lower_layer_holds_new_name_as_directory: bool,
    /// Holds the mode the real rename is issued with: the physical form of the caller's request.
    mode: RenameMode,
    whiteout: WhiteoutAction,
    new_name: &'x str,
    old_parent: Arc<OverlayInode>,
    old_parent_dentry: Arc<Dentry>,
    new_parent: Arc<OverlayInode>,
    new_parent_dentry: &'x Dentry,
    source: Arc<OverlayInode>,
    source_dentry: &'x Dentry,
    target: RenameTarget,
}

impl OverlayInode {
    /// Serves as the rename entry the VFS calls: it freezes the facts of both names, decides the
    /// move they allow, and performs it.
    pub(in super::super) fn rename_impl(
        &self,
        old_child_dentry: &Dentry,
        new_dir_dentry: &Dentry,
        new_name: &str,
        replaced_inode: Option<&Arc<dyn Inode>>,
        mode: RenameMode,
    ) -> Result<()> {
        let facts = RenameFacts::classify(
            self.self_arc(),
            old_child_dentry,
            new_dir_dentry,
            new_name,
            replaced_inode,
            mode,
        )?;
        facts.probe_target_empty_early()?;

        facts.promote_members()?;
        let Some(mut locks) = RenameLocks::acquire(&facts)? else {
            return_errno!(Errno::ESTALE)
        };

        facts.sweep_replaced_whiteouts_if_needed(&mut locks)?;
        facts.set_opaque_marker_if_needed(&mut locks)?;
        facts.set_impure_marker_if_needed()?;
        facts.commit(&mut locks)
    }
}

impl<'x> RenameFacts<'x> {
    /// Builds the facts of one rename — the target name classified once and the two answers the
    /// lower layers give — together with the plan they decide.
    fn classify(
        old_parent: Arc<OverlayInode>,
        old_child_dentry: &'x Dentry,
        new_dir_dentry: &'x Dentry,
        new_name: &'x str,
        replaced_inode: Option<&'x Arc<dyn Inode>>,
        request: RenameMode,
    ) -> Result<Self> {
        let old_parent_dentry = old_child_dentry
            .parent()
            .expect("the renamed child has no parent dentry");
        let source = Arc::downcast::<OverlayInode>(old_child_dentry.inode().clone())
            .expect("the rename source is not an overlay inode");
        let new_parent = Arc::downcast::<OverlayInode>(new_dir_dentry.inode().clone())
            .expect("the rename target parent is not an overlay inode");
        let fs = old_parent.fs_arc();
        let handed_inode = replaced_inode.map(|inode| {
            Arc::downcast::<OverlayInode>((*inode).clone())
                .expect("the rename target is not an overlay inode")
        });
        let target = match &handed_inode {
            Some(inode) => RenameTarget::Visible {
                inode: inode.clone(),
            },
            None => {
                // The name the VFS found free is held by an upper whiteout or by nothing at all.
                let upper_dir = new_parent
                    .upper
                    .get()
                    .map(|upper| upper.dentry().as_dir_dentry_or_err())
                    .transpose()?;
                let upper_holds_whiteout = match upper_dir {
                    Some(dir) => match dir.lookup_child(new_name) {
                        Ok(child) => fs.is_whiteout(&child)?,
                        Err(err) if err.error() == Errno::ENOENT => false,
                        Err(err) => return Err(err),
                    },
                    None => false,
                };
                if upper_holds_whiteout {
                    RenameTarget::UpperWhiteout
                } else {
                    RenameTarget::Absent
                }
            }
        };

        // An unanswerable probe counts as a hit, so the rename never acts on an unknown.
        let lower_layer_holds_old_name = match old_parent.lower_entry(&old_child_dentry.name()) {
            Ok(found) => found.is_some(),
            Err(_) => true,
        };
        let lower_layer_holds_new_name_as_directory = match new_parent.lower_entry(new_name) {
            Ok(found) => found.is_some_and(|type_| type_.is_directory()),
            Err(_) => true,
        };
        let (mode, whiteout) = Self::decide_rename_plan(
            request,
            &source,
            &target,
            lower_layer_holds_old_name,
            lower_layer_holds_new_name_as_directory,
            fs.policy(),
        )?;

        Ok(Self {
            lower_layer_holds_new_name_as_directory,
            mode,
            whiteout,
            new_name,
            old_parent,
            old_parent_dentry,
            new_parent,
            new_parent_dentry: new_dir_dentry,
            source,
            source_dentry: old_child_dentry,
            target,
        })
    }

    /// Decides the rename plan the facts allow: the refusals taken before any lock, the mode the
    /// real rename is issued with, and the old name's whiteout verdict.
    fn decide_rename_plan(
        request: RenameMode,
        source: &OverlayInode,
        target: &RenameTarget,
        lower_layer_holds_old_name: bool,
        lower_layer_holds_new_name_as_directory: bool,
        policy: &MountPolicy,
    ) -> Result<(RenameMode, WhiteoutAction)> {
        let source_is_dir = source.type_().is_directory();
        let source_is_merged_dir = source_is_dir && !source.lowers.is_empty();
        let target_is_dir = matches!(
            target,
            RenameTarget::Visible { inode } if inode.type_().is_directory()
        );
        let target_is_visible = matches!(target, RenameTarget::Visible { .. });
        let target_is_merged_dir = target_is_dir
            && matches!(target, RenameTarget::Visible { inode } if !inode.lowers.is_empty());
        if source_is_merged_dir
            || (request == RenameMode::Exchange && target_is_dir && target_is_merged_dir)
        {
            return Err(Error::with_message(
                Errno::EXDEV,
                "the overlay cross-directory rename of a lower-backed or merged directory \
                 requires the not-yet-implemented redirect_dir policy",
            ));
        }
        if request != RenameMode::Exchange && source_is_dir && target_is_visible && !target_is_dir {
            return Err(Error::with_message(
                Errno::ENOTDIR,
                "a directory cannot be renamed onto a non-directory name",
            ));
        }
        if request != RenameMode::Exchange && !source_is_dir && target_is_visible && target_is_dir {
            return Err(Error::with_message(
                Errno::EISDIR,
                "a non-directory cannot be renamed onto a directory name",
            ));
        }
        if request == RenameMode::NoReplace && target_is_visible {
            return Err(Error::with_message(
                Errno::EEXIST,
                "the rename target already exists and is visible",
            ));
        }

        let target_whiteout_is_swapped = matches!(target, RenameTarget::UpperWhiteout)
            && (source_is_dir || lower_layer_holds_old_name);
        let mode = if request == RenameMode::Exchange || target_whiteout_is_swapped {
            RenameMode::Exchange
        } else {
            RenameMode::Replace
        };
        // An exchange swaps the two names, so the target's whiteout travels with its name.
        let whiteout = if request == RenameMode::Exchange {
            WhiteoutAction::None
        } else {
            match target {
                RenameTarget::UpperWhiteout if lower_layer_holds_old_name => {
                    WhiteoutAction::Exchange
                }
                RenameTarget::UpperWhiteout if source_is_dir => WhiteoutAction::ExchangeThenDelete,
                RenameTarget::UpperWhiteout => WhiteoutAction::None,
                RenameTarget::Absent | RenameTarget::Visible { .. } => {
                    if lower_layer_holds_old_name {
                        WhiteoutAction::Create
                    } else {
                        WhiteoutAction::None
                    }
                }
            }
        };

        if whiteout == WhiteoutAction::Create && !policy.can_express_whiteout() {
            return Err(Error::with_message(
                Errno::EOPNOTSUPP,
                "the upper filesystem supports no whiteout form; the rename cannot publish one",
            ));
        }
        if source_is_dir
            && lower_layer_holds_new_name_as_directory
            && !policy.can_store_private_xattr()
        {
            return Err(Error::with_message(
                Errno::EOPNOTSUPP,
                "the upper filesystem cannot store the opaque marker required for a rename",
            ));
        }
        Ok((mode, whiteout))
    }

    /// Returns the emptiness verdict taken before the lock set: a directory source replacing a
    /// visible directory must find it empty, the only gate when the target has no upper entry.
    fn probe_target_empty_early(&self) -> Result<()> {
        if !self.needs_empty_check() {
            return Ok(());
        }
        let RenameTarget::Visible { inode, .. } = &self.target else {
            return Ok(());
        };
        let target_guard = inode.lock();
        if inode.is_empty_dir(&target_guard)? {
            return Ok(());
        }
        Err(Error::with_message(
            Errno::ENOTEMPTY,
            "the overlay rename target directory is not empty",
        ))
    }

    /// Promotes the objects this move will touch — both parents, the source, and an exchange's
    /// target without an upper.
    ///
    /// The real rename can be expressed on the upper alone.
    fn promote_members(&self) -> Result<()> {
        self.old_parent
            .writable_real_object(&self.old_parent_dentry)?;
        self.new_parent
            .writable_real_object(self.new_parent_dentry)?;
        self.source.writable_real_object(self.source_dentry)?;
        let RenameTarget::Visible { inode } = &self.target else {
            return Ok(());
        };
        if self.mode != RenameMode::Exchange || inode.upper.get().is_some() {
            return Ok(());
        }
        let fs = self.old_parent.fs_arc();
        fs.promote_rename_target(&self.new_parent, inode, self.new_name)?;
        Ok(())
    }

    /// Clears the whiteout residue out of the upper directory this move replaces.
    fn sweep_replaced_whiteouts_if_needed(&self, _locks: &mut RenameLocks<'_>) -> Result<()> {
        if self.mode == RenameMode::Exchange {
            return Ok(());
        }
        let RenameTarget::Visible { inode, .. } = &self.target else {
            return Ok(());
        };
        if !inode.type_().is_directory() {
            return Ok(());
        }
        let Some(target_upper_dir) = inode.upper.get() else {
            return Ok(());
        };
        self.old_parent
            .fs_arc()
            .sweep_whiteouts(target_upper_dir.dentry())
    }

    /// Writes the opaque record on the source's upper entry.
    ///
    /// The moved directory's new name stops merging a lower directory into it.
    fn set_opaque_marker_if_needed(&self, _locks: &mut RenameLocks<'_>) -> Result<()> {
        if !self.needs_opaque() {
            return Ok(());
        }
        let fs = self.old_parent.fs_arc();
        let namespace = fs.policy().xattr_namespace();
        let source_upper = self.source.writable_upper();
        OverlayXattrType::Opaque.set_value_on(source_upper.dentry(), namespace, None)
    }

    /// Writes the impure record on the new parent from either origin direction — the source's own
    /// or, for an exchange, the target's.
    ///
    /// A failure here still stops the move before it commits.
    fn set_impure_marker_if_needed(&self) -> Result<()> {
        if !self.is_cross_parent() {
            return Ok(());
        }
        let fs = self.old_parent.fs_arc();
        let source_has_origin = fs
            .origin_of(self.source.writable_upper().dentry())
            .is_some();
        let target_has_origin = match &self.target {
            RenameTarget::Visible { inode, .. } if self.mode == RenameMode::Exchange => {
                fs.origin_of(inode.writable_upper().dentry()).is_some()
            }
            _ => false,
        };
        if !source_has_origin && !target_has_origin {
            return Ok(());
        }
        if !fs.policy().can_store_private_xattr() {
            return Err(Error::with_message(
                Errno::EOPNOTSUPP,
                "the upper filesystem cannot store the impure marker required for a rename",
            ));
        }
        let namespace = fs.policy().xattr_namespace();
        let new_parent_upper = self.new_parent.writable_upper().dentry();
        if OverlayXattrType::Impure.is_positive_on(new_parent_upper, namespace)? {
            return Ok(());
        }
        OverlayXattrType::Impure.set_value_on(new_parent_upper, namespace, None)
    }

    /// Commits the move: the real rename in the upper, then the old name's whiteout action.
    ///
    /// The consequences the rename already fixed — the displaced target's latch and each parent's
    /// snapshot drop — hold whether or not the whiteout step succeeded; its error is reported, not
    /// rolled back.
    fn commit(&self, locks: &mut RenameLocks<'_>) -> Result<()> {
        let old_name = self.source_dentry.name();
        let upper_parent = self.old_parent.writable_upper().dentry();
        let new_upper_parent = self.new_parent.writable_upper().dentry();
        upper_parent.as_dir_dentry_or_err()?.rename(
            &old_name,
            &new_upper_parent.as_dir_dentry_or_err()?,
            self.new_name,
            self.mode,
        )?;

        let fs = self.old_parent.fs_arc();
        let outcome = match self.whiteout {
            WhiteoutAction::None | WhiteoutAction::Exchange => Ok(()),
            WhiteoutAction::Create => fs.publish_whiteout(upper_parent, &old_name, None),
            WhiteoutAction::ExchangeThenDelete => fs.remove_whiteout(upper_parent, &old_name),
        };

        if self.mode == RenameMode::Replace
            && let RenameTarget::Visible { inode, .. } = &self.target
        {
            inode.mark_name_taken();
        }
        *locks.old_parent_guard = None;
        if let Some(new_parent_guard) = locks.new_parent_guard.as_deref_mut() {
            *new_parent_guard = None;
        }
        outcome
    }

    fn source_is_dir(&self) -> bool {
        self.source.type_().is_directory()
    }

    fn is_cross_parent(&self) -> bool {
        !self.old_parent.same_directory_as(&self.new_parent)
    }

    fn needs_empty_check(&self) -> bool {
        if self.mode == RenameMode::Exchange {
            return false;
        }
        let RenameTarget::Visible { inode, .. } = &self.target else {
            return false;
        };
        self.source_is_dir() && inode.type_().is_directory()
    }

    fn needs_opaque(&self) -> bool {
        self.source_is_dir() && self.lower_layer_holds_new_name_as_directory
    }
}

/// Records the role a member plays in the lock set.
///
/// It makes the set readable by role and derives its key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LockRole {
    OldParent,
    NewParent,
    Target,
}

/// Holds the lock set of one rename: both parents, plus the target of an overwrite, taken in one
/// pass in ascending `(depth, address)` order, handed to a step inside it as a token, not a guard.
struct RenameLocks<'a> {
    old_parent_guard: OverlayInodeLockGuard<'a>,
    new_parent_guard: Option<OverlayInodeLockGuard<'a>>,
    #[expect(
        dead_code,
        reason = "the guard is held so the overwritten target is latched under its own lock"
    )]
    target_guard: Option<OverlayInodeLockGuard<'a>>,
}

impl<'a> RenameLocks<'a> {
    /// Returns the key that orders one lock-set member: the depth of the directory that hosts the
    /// object, then the object's own address.
    fn member_key(
        role: LockRole,
        old_parent_depth: usize,
        new_parent_depth: usize,
        address: usize,
    ) -> (usize, usize) {
        let depth = match role {
            LockRole::OldParent => old_parent_depth,
            LockRole::NewParent => new_parent_depth,
            LockRole::Target => new_parent_depth + 1,
        };
        (depth, address)
    }

    /// Counts the dcache parent steps from this dentry to the mount root.
    fn dentry_depth(dentry: &Dentry) -> usize {
        let mut depth = 0;
        let mut current = dentry.parent();
        while let Some(parent) = current {
            depth += 1;
            current = parent.parent();
        }
        depth
    }

    /// Takes the lock set in ascending key order, then re-checks both name bindings under it.
    ///
    /// The facts predate the set, so `Ok(None)` is stale and the caller answers `ESTALE`.
    fn acquire(facts: &'a RenameFacts<'_>) -> Result<Option<Self>> {
        let fs = facts.old_parent.fs_arc();
        let old_parent_depth = Self::dentry_depth(&facts.old_parent_dentry);
        let new_parent_depth = Self::dentry_depth(facts.new_parent_dentry);
        let same_parent = Arc::ptr_eq(&facts.old_parent, &facts.new_parent);
        let target_member = match &facts.target {
            RenameTarget::Visible { inode, .. } if facts.mode == RenameMode::Replace => {
                Some(inode.as_ref())
            }
            _ => None,
        };

        let old_parent_address = core::ptr::from_ref(facts.old_parent.as_ref()) as usize;
        let new_parent_address = core::ptr::from_ref(facts.new_parent.as_ref()) as usize;
        let target_address = target_member.map(|object| core::ptr::from_ref(object) as usize);
        let mut ordered: [((usize, usize), LockRole); 3] = [
            ((usize::MAX, usize::MAX), LockRole::OldParent),
            ((usize::MAX, usize::MAX), LockRole::NewParent),
            ((usize::MAX, usize::MAX), LockRole::Target),
        ];
        for slot in &mut ordered {
            let role = slot.1;
            let member_address = match role {
                LockRole::OldParent => Some(old_parent_address),
                LockRole::NewParent => (!same_parent).then_some(new_parent_address),
                LockRole::Target => target_address,
            };
            let Some(address) = member_address else {
                continue;
            };
            *slot = (
                Self::member_key(role, old_parent_depth, new_parent_depth, address),
                role,
            );
        }
        for index in 1..3 {
            let mut position = index;
            while position > 0 && ordered[position - 1].0 > ordered[position].0 {
                ordered.swap(position - 1, position);
                position -= 1;
            }
        }

        let mut old_parent_guard = None;
        let mut new_parent_guard = None;
        let mut target_guard = None;
        for (_, role) in ordered {
            match role {
                LockRole::OldParent => old_parent_guard = Some(facts.old_parent.lock()),
                LockRole::NewParent => {
                    if !same_parent {
                        new_parent_guard = Some(facts.new_parent.lock());
                    }
                }
                LockRole::Target => {
                    if let Some(object) = target_member {
                        target_guard = Some(object.lock());
                    }
                }
            }
        }
        let Some(old_parent_guard) = old_parent_guard else {
            unreachable!("the rename lock set always holds the old parent");
        };

        let source_is_fresh = match fs.lookup(&facts.old_parent, &facts.source_dentry.name())? {
            Lookup::Positive(rebound) => Arc::ptr_eq(&rebound, &facts.source),
            Lookup::Negative(_) => false,
        };
        let fresh = fs.lookup(&facts.new_parent, facts.new_name)?;
        let upper_whiteout = match &fresh {
            Lookup::Negative(NegativeLookup::HiddenByWhiteout) => {
                let upper_dir = facts
                    .new_parent
                    .upper
                    .get()
                    .map(|upper| upper.dentry().as_dir_dentry_or_err())
                    .transpose()?;
                match upper_dir {
                    Some(dir) => match dir.lookup_child(facts.new_name) {
                        Ok(child) => fs.is_whiteout(&child)?,
                        Err(err) if err.error() == Errno::ENOENT => false,
                        Err(err) => return Err(err),
                    },
                    None => false,
                }
            }
            Lookup::Negative(NegativeLookup::Absent) | Lookup::Positive(_) => false,
        };
        let target_is_fresh = match (&facts.target, &fresh) {
            (RenameTarget::Visible { inode, .. }, Lookup::Positive(rebound)) => {
                Arc::ptr_eq(inode, rebound)
            }
            (RenameTarget::Absent, Lookup::Negative(NegativeLookup::Absent)) => true,
            (RenameTarget::Absent, Lookup::Negative(NegativeLookup::HiddenByWhiteout)) => {
                !upper_whiteout
            }
            (RenameTarget::UpperWhiteout, Lookup::Negative(NegativeLookup::HiddenByWhiteout)) => {
                upper_whiteout
            }
            _ => false,
        };
        if !source_is_fresh || !target_is_fresh {
            return Ok(None);
        }

        Ok(Some(Self {
            old_parent_guard,
            new_parent_guard,
            target_guard,
        }))
    }
}
