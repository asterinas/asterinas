// SPDX-License-Identifier: MPL-2.0

use super::{
    attachment::AppArmorAttachment,
    capability::AppArmorCapabilityPolicy,
    path::{
        AppArmorExecMode, AppArmorExecTransition, AppArmorFilePermission, AppArmorPathPattern,
        AppArmorPathRule,
    },
    policy_update::AppArmorPolicyUpdate,
    profile::{
        AppArmorFilePolicy, AppArmorProfile, AppArmorProfileName, AppArmorProfileTransitionPolicy,
    },
    state::AppArmorMode,
};
use crate::{prelude::*, process::credentials::capabilities::CapSet};

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
    let mut file_rules = Vec::new();
    let mut allowed_capabilities = CapSet::empty();
    let mut change_profile_targets = Vec::new();
    let mut change_onexec_targets = Vec::new();
    for line in lines {
        match parse_rule(line)? {
            AppArmorTextRule::File(rule) => file_rules.push(rule),
            AppArmorTextRule::Capability(capabilities) => allowed_capabilities |= capabilities,
            AppArmorTextRule::ChangeProfile(profile_name) => {
                change_profile_targets.push(profile_name);
            }
            AppArmorTextRule::ChangeOnexec(profile_name) => {
                change_onexec_targets.push(profile_name);
            }
        }
    }

    let attachment = AppArmorAttachment::from_profile(&profile_name, None, None);
    Ok(AppArmorPolicyUpdate::Replace(Box::new(
        AppArmorProfile::new_with_transition_policy(
            profile_name,
            attachment,
            mode,
            AppArmorFilePolicy::PathRules(file_rules),
            AppArmorCapabilityPolicy::new(allowed_capabilities, CapSet::empty(), CapSet::empty()),
            AppArmorProfileTransitionPolicy::new(change_profile_targets, change_onexec_targets),
        ),
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

enum AppArmorTextRule {
    File(AppArmorPathRule),
    Capability(CapSet),
    ChangeProfile(AppArmorProfileName),
    ChangeOnexec(AppArmorProfileName),
}

fn parse_rule(line: &str) -> Result<AppArmorTextRule> {
    let mut tokens = line.split_whitespace();
    let Some(rule_kind) = tokens.next() else {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor rule is empty");
    };
    let deny = match rule_kind {
        "allow" => false,
        "deny" => true,
        _ => return_errno_with_message!(Errno::EINVAL, "the AppArmor rule kind is invalid"),
    };

    if tokens.clone().next() == Some("capability") {
        return parse_capability_rule(tokens, deny);
    }
    if tokens.clone().next() == Some("change_profile") {
        return parse_profile_transition_rule(tokens, deny, true);
    }
    if tokens.clone().next() == Some("change_onexec") {
        return parse_profile_transition_rule(tokens, deny, false);
    }

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

    Ok(AppArmorTextRule::File(
        AppArmorPathRule::new_with_transition(pattern, permissions, exec_transition, audit, deny),
    ))
}

fn parse_profile_transition_rule<'a>(
    mut tokens: impl Iterator<Item = &'a str>,
    deny: bool,
    immediate: bool,
) -> Result<AppArmorTextRule> {
    if deny {
        return_errno_with_message!(
            Errno::EINVAL,
            "deny profile transition rules are not supported by the text AppArmor loader"
        );
    }

    let expected = if immediate {
        "change_profile"
    } else {
        "change_onexec"
    };
    if tokens.next() != Some(expected) {
        return_errno_with_message!(
            Errno::EINVAL,
            "the AppArmor profile transition rule is invalid"
        );
    }

    let Some(profile_name) = tokens.next() else {
        return_errno_with_message!(
            Errno::EINVAL,
            "the AppArmor profile transition target is missing"
        );
    };
    if tokens.next().is_some() {
        return_errno_with_message!(
            Errno::EINVAL,
            "the AppArmor profile transition rule has extra fields"
        );
    }

    let profile_name = AppArmorProfileName::new(profile_name.to_string())?;
    if immediate {
        Ok(AppArmorTextRule::ChangeProfile(profile_name))
    } else {
        Ok(AppArmorTextRule::ChangeOnexec(profile_name))
    }
}

fn parse_capability_rule<'a>(
    mut tokens: impl Iterator<Item = &'a str>,
    deny: bool,
) -> Result<AppArmorTextRule> {
    if deny {
        return_errno_with_message!(
            Errno::EINVAL,
            "deny capability rules are not supported by the text AppArmor loader"
        );
    }
    if tokens.next() != Some("capability") {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor capability rule is invalid");
    }

    let Some(capabilities) = tokens.next() else {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor capability rule is missing");
    };
    if tokens.next().is_some() {
        return_errno_with_message!(
            Errno::EINVAL,
            "the AppArmor capability rule has extra fields"
        );
    }

    Ok(AppArmorTextRule::Capability(parse_capabilities(
        capabilities,
    )?))
}

fn parse_capabilities(capabilities_text: &str) -> Result<CapSet> {
    let mut capabilities = CapSet::empty();

    for capability_text in capabilities_text.split(',') {
        if capability_text.is_empty() {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor capability is empty");
        }
        capabilities |= parse_capability_token(capability_text)?;
    }

    if capabilities.is_empty() {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor capabilities are empty");
    }

    Ok(capabilities)
}

fn parse_capability_token(capability_text: &str) -> Result<CapSet> {
    let capability = match capability_text {
        "all" => CapSet::all(),
        "chown" => CapSet::CHOWN,
        "dac_override" => CapSet::DAC_OVERRIDE,
        "dac_read_search" => CapSet::DAC_READ_SEARCH,
        "fowner" => CapSet::FOWNER,
        "fsetid" => CapSet::FSETID,
        "kill" => CapSet::KILL,
        "setgid" => CapSet::SETGID,
        "setuid" => CapSet::SETUID,
        "setpcap" => CapSet::SETPCAP,
        "linux_immutable" => CapSet::LINUX_IMMUTABLE,
        "net_bind_service" => CapSet::NET_BIND_SERVICE,
        "net_broadcast" => CapSet::NET_BROADCAST,
        "net_admin" => CapSet::NET_ADMIN,
        "net_raw" => CapSet::NET_RAW,
        "ipc_lock" => CapSet::IPC_LOCK,
        "ipc_owner" => CapSet::IPC_OWNER,
        "sys_module" => CapSet::SYS_MODULE,
        "sys_rawio" => CapSet::SYS_RAWIO,
        "sys_chroot" => CapSet::SYS_CHROOT,
        "sys_ptrace" => CapSet::SYS_PTRACE,
        "sys_pacct" => CapSet::SYS_PACCT,
        "sys_admin" => CapSet::SYS_ADMIN,
        "sys_boot" => CapSet::SYS_BOOT,
        "sys_nice" => CapSet::SYS_NICE,
        "sys_resource" => CapSet::SYS_RESOURCE,
        "sys_time" => CapSet::SYS_TIME,
        "sys_tty_config" => CapSet::SYS_TTY_CONFIG,
        "mknod" => CapSet::MKNOD,
        "lease" => CapSet::LEASE,
        "audit_write" => CapSet::AUDIT_WRITE,
        "audit_control" => CapSet::AUDIT_CONTROL,
        "setfcap" => CapSet::SETFCAP,
        "mac_override" => CapSet::MAC_OVERRIDE,
        "mac_admin" => CapSet::MAC_ADMIN,
        "syslog" => CapSet::SYSLOG,
        "wake_alarm" => CapSet::WAKE_ALARM,
        "block_suspend" => CapSet::BLOCK_SUSPEND,
        "audit_read" => CapSet::AUDIT_READ,
        "perfmon" => CapSet::PERFMON,
        "bpf" => CapSet::BPF,
        "checkpoint_restore" => CapSet::CHECKPOINT_RESTORE,
        _ => return_errno_with_message!(Errno::EINVAL, "the AppArmor capability is invalid"),
    };

    Ok(capability)
}

fn parse_exec_transition(token: &str) -> Result<Option<AppArmorExecTransition>> {
    match token {
        "ix" => Ok(Some(AppArmorExecTransition::Inherit)),
        "ux" => Ok(Some(AppArmorExecTransition::unconfined(
            AppArmorExecMode::Unsafe,
        ))),
        "Ux" => Ok(Some(AppArmorExecTransition::unconfined(
            AppArmorExecMode::Safe,
        ))),
        _ => {
            let transition = if let Some(profile_name) = token.strip_prefix("px:") {
                AppArmorExecTransition::profile(
                    AppArmorProfileName::new(profile_name.to_string())?,
                    AppArmorExecMode::Unsafe,
                )
            } else if let Some(profile_name) = token.strip_prefix("Px:") {
                AppArmorExecTransition::profile(
                    AppArmorProfileName::new(profile_name.to_string())?,
                    AppArmorExecMode::Safe,
                )
            } else if let Some(profile_name) = token.strip_prefix("cx:") {
                AppArmorExecTransition::child(
                    AppArmorProfileName::new(profile_name.to_string())?,
                    AppArmorExecMode::Unsafe,
                )
            } else if let Some(profile_name) = token.strip_prefix("Cx:") {
                AppArmorExecTransition::child(
                    AppArmorProfileName::new(profile_name.to_string())?,
                    AppArmorExecMode::Safe,
                )
            } else {
                return Ok(None);
            };
            Ok(Some(transition))
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
