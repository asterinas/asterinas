// SPDX-License-Identifier: MPL-2.0

//! AppArmor labels and profile-load identities.
//!
//! Asterinas currently supports only the root AppArmor policy namespace.
//! Profile-load identities are therefore meaningful only within that namespace.
//! Hierarchical namespaces must add namespace identity to label equality before being introduced.

use core::num::NonZeroU64;

use super::profile::AppArmorProfileName;
use crate::prelude::*;

/// An identity for one load lifetime of an AppArmor profile name.
///
/// Replacing a profile preserves this identity.
/// Removing and loading the same name creates a new identity,
/// preventing old labels from being rebound to new policy accidentally.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct AppArmorProfileIdentity(NonZeroU64);

impl AppArmorProfileIdentity {
    #[cfg_attr(
        not(ktest),
        expect(dead_code, reason = "used by the policy namespace introduced in PR2")
    )]
    pub(super) const fn new(value: NonZeroU64) -> Self {
        Self(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum AppArmorLabelKind {
    Unconfined {
        profile_name: AppArmorProfileName,
    },
    Confined {
        profile_name: AppArmorProfileName,
        identity: AppArmorProfileIdentity,
    },
}

/// An AppArmor profile label attached to a task or object.
///
/// The supported model carries one profile per label.
/// Profile stacking requires a separate representation with explicit ordering and scope invariants.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AppArmorLabel {
    kind: AppArmorLabelKind,
}

impl AppArmorLabel {
    /// Creates an unconfined label.
    pub(super) fn new_unconfined() -> Self {
        Self {
            kind: AppArmorLabelKind::Unconfined {
                profile_name: AppArmorProfileName::new_unconfined(),
            },
        }
    }

    /// Creates a label containing a single confined profile.
    #[cfg_attr(
        not(ktest),
        expect(dead_code, reason = "used by the policy namespace introduced in PR2")
    )]
    pub(super) fn new_single(
        profile_name: AppArmorProfileName,
        identity: AppArmorProfileIdentity,
    ) -> Result<Self> {
        if profile_name.is_unconfined() {
            return_errno_with_message!(
                Errno::EINVAL,
                "a confined AppArmor label cannot use the unconfined profile name"
            );
        }

        Ok(Self {
            kind: AppArmorLabelKind::Confined {
                profile_name,
                identity,
            },
        })
    }

    /// Returns the profile in this label.
    #[cfg_attr(
        not(ktest),
        expect(dead_code, reason = "used by label transitions introduced in PR3")
    )]
    pub(super) fn profile_name(&self) -> &AppArmorProfileName {
        match &self.kind {
            AppArmorLabelKind::Unconfined { profile_name } => profile_name,
            AppArmorLabelKind::Confined { profile_name, .. } => profile_name,
        }
    }

    /// Returns whether this is the unconfined label.
    #[cfg_attr(
        not(ktest),
        expect(dead_code, reason = "used by access checks introduced in PR4")
    )]
    pub(super) fn is_unconfined(&self) -> bool {
        matches!(self.kind, AppArmorLabelKind::Unconfined { .. })
    }

    /// Returns the load identity carried by this label.
    #[cfg_attr(
        not(ktest),
        expect(dead_code, reason = "used by the policy namespace introduced in PR2")
    )]
    pub(super) const fn identity(&self) -> Option<AppArmorProfileIdentity> {
        match &self.kind {
            AppArmorLabelKind::Unconfined { .. } => None,
            AppArmorLabelKind::Confined { identity, .. } => Some(*identity),
        }
    }
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::*;

    use super::*;

    fn label(name: &str, identity: u64) -> AppArmorLabel {
        AppArmorLabel::new_single(
            AppArmorProfileName::new(name.to_string()).unwrap(),
            AppArmorProfileIdentity::new(NonZeroU64::new(identity).unwrap()),
        )
        .unwrap()
    }

    #[ktest]
    fn new_unconfined_label_has_no_load_identity() {
        let label = AppArmorLabel::new_unconfined();

        assert!(label.is_unconfined());
        assert_eq!(label.profile_name().as_str(), "unconfined");
        assert_eq!(label.identity(), None);
    }

    #[ktest]
    fn cloning_a_label_preserves_load_identity() {
        let original = label("profile", 1);

        assert_eq!(original.clone(), original);
    }

    #[ktest]
    fn same_named_labels_from_different_loads_are_distinct() {
        assert_ne!(label("profile", 1), label("profile", 2));
    }

    #[ktest]
    fn rejects_confined_label_with_unconfined_name() {
        assert!(
            AppArmorLabel::new_single(
                AppArmorProfileName::new_unconfined(),
                AppArmorProfileIdentity::new(NonZeroU64::new(1).unwrap())
            )
            .is_err()
        );
    }
}
