// SPDX-License-Identifier: MPL-2.0

//! AppArmor profile names.

use crate::prelude::*;

/// The name of an AppArmor profile.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct AppArmorProfileName(Arc<str>);

impl AppArmorProfileName {
    /// The name of the implicit unconfined profile.
    const UNCONFINED: &'static str = "unconfined";

    /// Creates a profile name.
    #[cfg_attr(
        not(ktest),
        expect(dead_code, reason = "used by policy loading introduced in PR2")
    )]
    pub(super) fn new(name: String) -> Result<Self> {
        if name.is_empty() {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor profile name is empty");
        }
        // Asterinas initially supports only root-namespace profiles.
        // It does not expand variables and does not support hats.
        // The upstream policy language introduces these constructs with `:`, `@`, and `^`:
        // https://gitlab.com/apparmor/apparmor/-/blob/master/parser/apparmor.d.pod
        if name.starts_with([':', '@', '^']) {
            return_errno_with_message!(
                Errno::EINVAL,
                "the AppArmor profile name starts with a reserved prefix"
            );
        }
        if name.contains('\0') {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor profile name contains NUL");
        }

        Ok(Self(name.into()))
    }

    /// Creates the implicit unconfined profile name.
    pub(super) fn new_unconfined() -> Self {
        Self(Self::UNCONFINED.into())
    }

    /// Returns whether this is the implicit unconfined profile.
    pub(super) fn is_unconfined(&self) -> bool {
        self.as_str() == Self::UNCONFINED
    }

    /// Returns the profile name text.
    pub(super) fn as_str(&self) -> &str {
        self.0.as_ref()
    }
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::*;

    use super::*;

    #[ktest]
    fn rejects_empty_profile_names() {
        assert!(AppArmorProfileName::new(String::new()).is_err());
    }

    #[ktest]
    fn rejects_reserved_profile_name_prefixes() {
        for name in [":namespace", "@variable", "^hat"] {
            assert!(AppArmorProfileName::new(name.to_string()).is_err());
        }
    }

    #[ktest]
    fn rejects_embedded_nul_in_profile_names() {
        assert!(AppArmorProfileName::new("prefix\0suffix".to_string()).is_err());
    }

    #[ktest]
    fn new_unconfined_returns_the_implicit_profile_name() {
        let profile_name = AppArmorProfileName::new_unconfined();

        assert!(profile_name.is_unconfined());
        assert_eq!(profile_name.as_str(), AppArmorProfileName::UNCONFINED);
    }
}
