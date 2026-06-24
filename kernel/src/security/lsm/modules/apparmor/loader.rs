// SPDX-License-Identifier: MPL-2.0

use super::{
    path::{AppArmorExecTransition, AppArmorFilePermission, AppArmorPathPattern, AppArmorPathRule},
    policy_update::AppArmorPolicyUpdate,
    profile::{AppArmorProfile, AppArmorProfileName},
    state::AppArmorMode,
};
use crate::prelude::*;

pub(super) fn parse_policy_load(policy_text: &str) -> Result<AppArmorPolicyUpdate> {
    let mut lines = policy_text
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'));

    let Some(header) = lines.next() else {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor policy text is empty");
    };

    if header.starts_with("remove ") {
        let profile_name = parse_remove_header(header)?;
        if lines.next().is_some() {
            return_errno_with_message!(
                Errno::EINVAL,
                "remove commands must not include policy rules"
            );
        }
        return Ok(AppArmorPolicyUpdate::Remove(profile_name));
    }

    let (profile_name, mode) = parse_profile_header(header)?;
    let mut rules = Vec::new();
    for line in lines {
        rules.push(parse_rule(line)?);
    }

    Ok(AppArmorPolicyUpdate::Replace(AppArmorProfile::new(
        profile_name,
        mode,
        rules,
    )))
}

fn parse_remove_header(header: &str) -> Result<AppArmorProfileName> {
    let mut tokens = header.split_whitespace();
    if tokens.next() != Some("remove") {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor command is invalid");
    }

    let Some(profile_name) = tokens.next() else {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor profile name is missing");
    };

    if tokens.next().is_some() {
        return_errno_with_message!(Errno::EINVAL, "the remove command has extra fields");
    }

    let profile_name = AppArmorProfileName::new(profile_name.to_string())?;
    if profile_name.is_unconfined() {
        return_errno_with_message!(
            Errno::EINVAL,
            "the implicit unconfined AppArmor profile cannot be removed"
        );
    }

    Ok(profile_name)
}

fn parse_profile_header(header: &str) -> Result<(AppArmorProfileName, AppArmorMode)> {
    let mut tokens = header.split_whitespace();
    if tokens.next() != Some("profile") {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor profile header is invalid");
    }

    let Some(profile_name) = tokens.next() else {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor profile name is missing");
    };
    let profile_name = AppArmorProfileName::new(profile_name.to_string())?;
    if profile_name.is_unconfined() {
        return_errno_with_message!(
            Errno::EINVAL,
            "the implicit unconfined AppArmor profile cannot be loaded"
        );
    }

    let Some(mode) = tokens.next() else {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor profile mode is missing");
    };
    let mode = AppArmorMode::parse(mode)?;

    if tokens.next().is_some() {
        return_errno_with_message!(
            Errno::EINVAL,
            "the AppArmor profile header has extra fields"
        );
    }

    Ok((profile_name, mode))
}

fn parse_rule(line: &str) -> Result<AppArmorPathRule> {
    let mut tokens = line.split_whitespace();
    let Some(rule_kind) = tokens.next() else {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor rule is empty");
    };
    let deny = match rule_kind {
        "allow" => false,
        "deny" => true,
        _ => return_errno_with_message!(Errno::EINVAL, "the AppArmor rule kind is invalid"),
    };

    let Some(pattern) = tokens.next() else {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor path pattern is missing");
    };
    let pattern = AppArmorPathPattern::new(pattern.to_string());

    let Some(permissions) = tokens.next() else {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor permissions are missing");
    };
    let permissions = parse_permissions(permissions)?;

    let mut audit = false;
    let mut exec_transition = AppArmorExecTransition::Inherit;
    for token in tokens {
        if token == "audit" {
            audit = true;
            continue;
        }

        let Some(new_transition) = parse_exec_transition(token)? else {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor rule option is invalid");
        };

        if exec_transition != AppArmorExecTransition::Inherit {
            return_errno_with_message!(
                Errno::EINVAL,
                "the AppArmor rule has multiple exec transitions"
            );
        }
        exec_transition = new_transition;
    }

    if deny && exec_transition != AppArmorExecTransition::Inherit {
        return_errno_with_message!(
            Errno::EINVAL,
            "deny rules cannot carry AppArmor exec transitions"
        );
    }

    if exec_transition != AppArmorExecTransition::Inherit
        && !permissions.contains(AppArmorFilePermission::EXECUTE)
    {
        return_errno_with_message!(
            Errno::EINVAL,
            "AppArmor exec transitions require execute permission"
        );
    }

    Ok(AppArmorPathRule::new_with_transition(
        pattern,
        permissions,
        exec_transition,
        audit,
        deny,
    ))
}

fn parse_exec_transition(token: &str) -> Result<Option<AppArmorExecTransition>> {
    match token {
        "ix" => Ok(Some(AppArmorExecTransition::Inherit)),
        "ux" => Ok(Some(AppArmorExecTransition::Unconfined)),
        _ => {
            let Some(profile_name) = token.strip_prefix("px:") else {
                return Ok(None);
            };
            Ok(Some(AppArmorExecTransition::Profile(
                AppArmorProfileName::new(profile_name.to_string())?,
            )))
        }
    }
}

fn parse_permissions(permissions_text: &str) -> Result<AppArmorFilePermission> {
    let mut permissions = AppArmorFilePermission::empty();

    for permission_text in permissions_text.split(',') {
        if permission_text.is_empty() {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor permission is empty");
        }
        permissions |= parse_permission_token(permission_text)?;
    }

    if permissions.is_empty() {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor permissions are empty");
    }

    Ok(permissions)
}

fn parse_permission_token(permission_text: &str) -> Result<AppArmorFilePermission> {
    let permissions = match permission_text {
        "all" => AppArmorFilePermission::all(),
        "read" => AppArmorFilePermission::READ,
        "write" => AppArmorFilePermission::WRITE,
        "execute" | "exec" => AppArmorFilePermission::EXECUTE,
        "append" => AppArmorFilePermission::APPEND,
        "mmap" => AppArmorFilePermission::MMAP,
        "create" => AppArmorFilePermission::CREATE,
        "delete" => AppArmorFilePermission::DELETE,
        "link" => AppArmorFilePermission::LINK,
        "rename" => AppArmorFilePermission::RENAME,
        "mkdir" => AppArmorFilePermission::MKDIR,
        "mknod" => AppArmorFilePermission::MKNOD,
        "symlink" => AppArmorFilePermission::SYMLINK,
        "setattr" => AppArmorFilePermission::SETATTR,
        _ => return parse_compact_permissions(permission_text),
    };

    Ok(permissions)
}

fn parse_compact_permissions(permission_text: &str) -> Result<AppArmorFilePermission> {
    let mut permissions = AppArmorFilePermission::empty();

    for permission in permission_text.chars() {
        permissions |= match permission {
            'r' => AppArmorFilePermission::READ,
            'w' => AppArmorFilePermission::WRITE,
            'x' => AppArmorFilePermission::EXECUTE,
            'a' => AppArmorFilePermission::APPEND,
            'm' => AppArmorFilePermission::MMAP,
            'c' => AppArmorFilePermission::CREATE,
            'd' => AppArmorFilePermission::DELETE,
            'l' => AppArmorFilePermission::LINK,
            'n' => AppArmorFilePermission::RENAME,
            'k' => AppArmorFilePermission::MKDIR,
            'b' => AppArmorFilePermission::MKNOD,
            's' => AppArmorFilePermission::SYMLINK,
            't' => AppArmorFilePermission::SETATTR,
            _ => {
                return_errno_with_message!(Errno::EINVAL, "the AppArmor permission is invalid");
            }
        };
    }

    Ok(permissions)
}
