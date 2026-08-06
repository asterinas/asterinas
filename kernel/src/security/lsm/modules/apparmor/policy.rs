// SPDX-License-Identifier: MPL-2.0

//! AppArmor policy parsing, representation, and storage.

use spin::Once;

use super::UNCONFINED_PROFILE_NAME;
use crate::{prelude::*, security::lsm::hooks::FileOpenAccess};

pub(super) const POLICY_VERSION: &str = "0";
const MAX_PROFILE_NAME_LEN_BYTES: usize = 128;
const MAX_RULE_PATH_LEN_BYTES: usize = 4096;
const MAX_RULES_PER_PROFILE: usize = 1024;

#[derive(Debug)]
struct Profile {
    name: String,
    file_rules: BTreeMap<String, FileOpenAccess>,
}

impl Profile {
    fn parse(policy: &str) -> Result<Self> {
        let mut lines = policy
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'));

        let Some(version_line) = lines.next() else {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor policy is empty");
        };
        let mut version_fields = version_line.split_ascii_whitespace();
        if version_fields.next() != Some("version")
            || version_fields.next() != Some(POLICY_VERSION)
            || version_fields.next().is_some()
        {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor policy version is invalid");
        }

        let Some(profile_line) = lines.next() else {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor profile header is missing");
        };
        let mut profile_fields = profile_line.split_ascii_whitespace();
        if profile_fields.next() != Some("profile") {
            return_errno_with_message!(
                Errno::EINVAL,
                "the AppArmor policy must contain a profile header"
            );
        }
        let Some(name) = profile_fields.next() else {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor profile name is missing");
        };
        if profile_fields.next().is_some()
            || name == UNCONFINED_PROFILE_NAME
            || !is_valid_profile_name(name)
        {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor profile name is invalid");
        }

        let mut file_rules = BTreeMap::<String, FileOpenAccess>::new();
        for (rule_index, line) in lines.enumerate() {
            if rule_index >= MAX_RULES_PER_PROFILE {
                return_errno_with_message!(
                    Errno::E2BIG,
                    "the AppArmor profile has too many file rules"
                );
            }

            let mut fields = line.split_ascii_whitespace();
            let Some(path) = fields.next() else {
                continue;
            };
            let Some(permissions) = fields.next() else {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "an AppArmor file rule is missing permissions"
                );
            };
            if fields.next().is_some() || !is_canonical_absolute_path(path) {
                return_errno_with_message!(Errno::EINVAL, "an AppArmor file rule is invalid");
            }

            let permissions = parse_file_permissions(permissions)?;
            file_rules
                .entry(path.to_string())
                .and_modify(|current| current.insert(permissions))
                .or_insert(permissions);
        }

        Ok(Self {
            name: name.to_string(),
            file_rules,
        })
    }

    fn allows(&self, path: &str, requested: FileOpenAccess) -> bool {
        self.file_rules
            .get(path)
            .is_some_and(|allowed| allowed.contains(requested))
    }
}

#[derive(Default)]
struct PolicySnapshot {
    profiles: BTreeMap<String, Arc<Profile>>,
}

fn policy_store() -> &'static RwLock<Arc<PolicySnapshot>> {
    static POLICY_STORE: Once<RwLock<Arc<PolicySnapshot>>> = Once::new();

    POLICY_STORE.call_once(|| RwLock::new(Arc::new(PolicySnapshot::default())))
}

pub(super) fn load_profile(policy: &str) -> Result<()> {
    update_profile(ProfileUpdate::Load, Profile::parse(policy)?)
}

pub(super) fn replace_profile(policy: &str) -> Result<()> {
    update_profile(ProfileUpdate::Replace, Profile::parse(policy)?)
}

pub(super) fn loaded_profile_names() -> Vec<String> {
    policy_store().read().profiles.keys().cloned().collect()
}

pub(super) fn stored_profile_name(name: &str) -> Option<Arc<str>> {
    let snapshot = policy_store().read();
    let (stored_name, _) = snapshot.profiles.get_key_value(name)?;
    Some(Arc::from(stored_name.as_str()))
}

pub(super) fn allows_file(profile_name: &str, path: &str, requested: FileOpenAccess) -> bool {
    let snapshot = policy_store().read().clone();
    snapshot
        .profiles
        .get(profile_name)
        .is_some_and(|profile| profile.allows(path, requested))
}

pub(super) fn is_valid_profile_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_PROFILE_NAME_LEN_BYTES
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

enum ProfileUpdate {
    Load,
    Replace,
}

fn update_profile(update: ProfileUpdate, profile: Profile) -> Result<()> {
    let mut current = policy_store().write();
    let profile_exists = current.profiles.contains_key(&profile.name);
    match update {
        ProfileUpdate::Load if profile_exists => {
            return_errno_with_message!(Errno::EEXIST, "the AppArmor profile is already loaded");
        }
        ProfileUpdate::Replace if !profile_exists => {
            return_errno_with_message!(Errno::ENOENT, "the AppArmor profile is not loaded");
        }
        ProfileUpdate::Load | ProfileUpdate::Replace => {}
    }

    let mut profiles = current.profiles.clone();
    profiles.insert(profile.name.clone(), Arc::new(profile));
    *current = Arc::new(PolicySnapshot { profiles });

    Ok(())
}

fn is_canonical_absolute_path(path: &str) -> bool {
    if path.is_empty() || path.len() > MAX_RULE_PATH_LEN_BYTES || !path.starts_with('/') {
        return false;
    }
    if path == "/" {
        return true;
    }
    if path.ends_with('/') {
        return false;
    }

    path.split('/')
        .skip(1)
        .all(|component| !component.is_empty() && component != "." && component != "..")
}

fn parse_file_permissions(permissions: &str) -> Result<FileOpenAccess> {
    if permissions.is_empty() {
        return_errno_with_message!(Errno::EINVAL, "AppArmor file permissions are empty");
    }

    let mut parsed = FileOpenAccess::empty();
    for permission in permissions.bytes() {
        match permission {
            b'r' => parsed.insert(FileOpenAccess::READ),
            b'w' => parsed.insert(FileOpenAccess::WRITE | FileOpenAccess::TRUNCATE),
            _ => {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "AppArmor file permissions contain an unsupported value"
                );
            }
        }
    }

    Ok(parsed)
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::Profile;
    use crate::security::lsm::hooks::FileOpenAccess;

    #[ktest]
    fn parses_exact_path_permissions() {
        let profile = Profile::parse(
            "version 0\n\
             profile test-profile\n\
             /read-only r\n\
             /state rw\n",
        )
        .unwrap();

        assert!(profile.allows("/read-only", FileOpenAccess::READ));
        assert!(!profile.allows("/read-only", FileOpenAccess::WRITE));
        assert!(profile.allows(
            "/state",
            FileOpenAccess::READ | FileOpenAccess::WRITE | FileOpenAccess::TRUNCATE
        ));
        assert!(!profile.allows("/missing", FileOpenAccess::READ));
    }

    #[ktest]
    fn rejects_noncanonical_paths_and_unknown_permissions() {
        assert!(Profile::parse("version 0\nprofile test\n/tmp/../secret r\n").is_err());
        assert!(Profile::parse("version 0\nprofile test\n/tmp/file x\n").is_err());
    }
}
