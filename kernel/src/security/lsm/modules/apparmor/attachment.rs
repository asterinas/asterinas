// SPDX-License-Identifier: MPL-2.0

use super::{
    dfa::AppArmorDfaFilePolicy,
    path::{AppArmorPathPattern, AppArmorPathView},
    profile::AppArmorProfileName,
};
use crate::prelude::*;

/// A profile attachment matcher.
///
/// Linux AppArmor uses profile attachments to choose a profile when an
/// executable image is committed. Asterinas keeps the matcher separate from
/// file mediation so path-based MAC remains independent from exec attachment.
#[derive(Clone, Debug)]
pub(super) struct AppArmorAttachment {
    pattern: Option<AppArmorPathPattern>,
    policy: Option<AppArmorDfaFilePolicy>,
}

impl AppArmorAttachment {
    /// Creates an attachment from optional text and DFA matchers.
    pub(super) fn new(
        pattern: Option<AppArmorPathPattern>,
        policy: Option<AppArmorDfaFilePolicy>,
    ) -> Self {
        Self { pattern, policy }
    }

    /// Creates the attachment implied by Linux profile syntax.
    pub(super) fn from_profile(
        profile_name: &AppArmorProfileName,
        attach: Option<String>,
        policy: Option<AppArmorDfaFilePolicy>,
    ) -> Self {
        let pattern = attach
            .or_else(|| {
                profile_name
                    .as_str()
                    .starts_with('/')
                    .then(|| profile_name.as_str().into())
            })
            .map(AppArmorPathPattern::new);

        Self::new(pattern, policy)
    }

    /// Returns whether the attachment selects a profile for `path_view`.
    pub(super) fn matches(&self, path_view: &AppArmorPathView) -> bool {
        if !path_view.is_reachable() {
            return false;
        }

        if let Some(policy) = &self.policy
            && policy.matches_path(path_view)
        {
            return true;
        }

        self.pattern
            .as_ref()
            .is_some_and(|pattern| pattern.matches(path_view))
    }
}
