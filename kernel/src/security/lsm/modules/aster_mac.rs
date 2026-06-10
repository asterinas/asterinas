// SPDX-License-Identifier: MPL-2.0

use alloc::boxed::ThinBox;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use super::super::{
    BprmCheckContext, BprmCommittedCredsContext, FileOpenContext, LsmFlags, LsmModule,
    hooks::{LsmAlienAccessHook, LsmBprmHook, LsmCapabilityHook, LsmFileHook, LsmInodeHook},
};
use crate::{
    fs::{
        file::StatusFlags,
        vfs::{
            inode::Inode,
            xattr::{XATTR_VALUE_MAX_LEN, XattrName},
        },
    },
    prelude::*,
};

pub(super) static ASTER_MAC_LSM: AsterMacLsm = AsterMacLsm;

const DEFAULT_TASK_LABEL: &str = "unconfined";
const DEFAULT_INODE_LABEL: &str = "unlabeled";
const MANAGED_LABEL_XATTR: &str = "security.aster_mac.label";
const MAX_LABEL_LEN: usize = 64;

/// Implements a small xattr-driven major MAC module.
pub(super) struct AsterMacLsm;

impl LsmModule for AsterMacLsm {
    fn name(&self) -> &'static str {
        "aster_mac"
    }

    fn flags(&self) -> LsmFlags {
        LsmFlags::LEGACY_MAJOR | LsmFlags::EXCLUSIVE
    }
}

impl LsmAlienAccessHook for AsterMacLsm {}

impl LsmCapabilityHook for AsterMacLsm {}

impl LsmBprmHook for AsterMacLsm {
    fn on_bprm_check_security(&self, context: &BprmCheckContext<'_>) -> Result<()> {
        let executable_inode = context.executable().inode();
        let rules =
            inode_security_state(executable_inode.as_ref()).rules(executable_inode.as_ref());
        if rules.contains(AsterMacPolicyRules::EXEC_DENY) {
            return_errno_with_message!(
                Errno::EACCES,
                "exec is denied by the Aster MAC LSM xattr policy"
            );
        }

        Ok(())
    }

    fn on_bprm_committed_creds(&self, context: &BprmCommittedCredsContext<'_>) -> Result<()> {
        let executable_inode = context.executable().inode();
        let label = inode_security_state(executable_inode.as_ref())
            .exec_label(executable_inode.as_ref())
            .unwrap_or_else(|| DEFAULT_TASK_LABEL.to_string());
        context.credentials().set_aster_mac_label(label);

        Ok(())
    }
}

impl LsmInodeHook for AsterMacLsm {}

impl LsmFileHook for AsterMacLsm {
    fn on_file_open(&self, context: &FileOpenContext<'_>) -> Result<()> {
        let inode = context.path().inode();
        let rules = inode_security_state(inode.as_ref()).rules(inode.as_ref());
        let status_flags = context.status_flags();

        if rules.contains(AsterMacPolicyRules::OPEN_DENY) {
            return_errno_with_message!(
                Errno::EACCES,
                "open is denied by the Aster MAC LSM xattr policy"
            );
        }
        if status_flags.contains(StatusFlags::O_PATH) {
            return Ok(());
        }
        if context.access_mode().is_readable() && rules.contains(AsterMacPolicyRules::READ_DENY) {
            return_errno_with_message!(
                Errno::EACCES,
                "read-open is denied by the Aster MAC LSM xattr policy"
            );
        }
        if context.access_mode().is_writable() && rules.contains(AsterMacPolicyRules::WRITE_DENY) {
            return_errno_with_message!(
                Errno::EACCES,
                "write-open is denied by the Aster MAC LSM xattr policy"
            );
        }

        Ok(())
    }
}

bitflags! {
    struct AsterMacPolicyRules: u8 {
        const OPEN_DENY  = 1 << 0;
        const READ_DENY  = 1 << 1;
        const WRITE_DENY = 1 << 2;
        const EXEC_DENY  = 1 << 3;
    }
}

struct AsterMacInodeSecurityState {
    rules: AtomicU8,
    has_label: AtomicBool,
    label: RwLock<String>,
    is_hydrated: AtomicBool,
}

impl AsterMacInodeSecurityState {
    fn new() -> Self {
        Self {
            rules: AtomicU8::new(0),
            has_label: AtomicBool::new(false),
            label: RwLock::new(String::new()),
            is_hydrated: AtomicBool::new(false),
        }
    }

    fn rules(&self, inode: &dyn Inode) -> AsterMacPolicyRules {
        self.ensure_hydrated(inode);
        AsterMacPolicyRules::from_bits_truncate(self.rules.load(Ordering::Relaxed))
    }

    fn update(&self, rule: AsterMacPolicyRules, enabled: bool) {
        if enabled {
            self.rules.fetch_or(rule.bits(), Ordering::Relaxed);
        } else {
            self.rules.fetch_and(!rule.bits(), Ordering::Relaxed);
        }
        self.is_hydrated.store(true, Ordering::Relaxed);
    }

    fn exec_label(&self, inode: &dyn Inode) -> Option<String> {
        self.ensure_hydrated(inode);
        self.has_label
            .load(Ordering::Relaxed)
            .then(|| self.label.read().clone())
    }

    fn set_label(&self, label: Option<String>) {
        self.has_label.store(label.is_some(), Ordering::Relaxed);
        let mut stored_label = self.label.write();
        *stored_label = label.unwrap_or_else(|| DEFAULT_INODE_LABEL.to_string());
        self.is_hydrated.store(true, Ordering::Relaxed);
    }

    fn ensure_hydrated(&self, inode: &dyn Inode) {
        if self.is_hydrated.load(Ordering::Acquire) {
            return;
        }

        let rules = load_inode_policy_rules(inode);
        let label = load_inode_label(inode);
        self.rules.store(rules.bits(), Ordering::Relaxed);
        self.has_label.store(label.is_some(), Ordering::Relaxed);
        *self.label.write() = label.unwrap_or_else(|| DEFAULT_INODE_LABEL.to_string());
        self.is_hydrated.store(true, Ordering::Release);
    }
}

fn inode_security_state(inode: &dyn Inode) -> &AsterMacInodeSecurityState {
    inode
        .extension()
        .group3()
        .call_once(|| ThinBox::new_unsize(AsterMacInodeSecurityState::new()))
        .downcast_ref()
        .unwrap()
}

pub(in crate::security) fn is_aster_mac_inode_xattr(name: &XattrName<'_>) -> bool {
    xattr_rule(name).is_some() || name.full_name() == MANAGED_LABEL_XATTR
}

pub(in crate::security) fn validate_aster_mac_inode_xattr(
    name: &XattrName<'_>,
    value: &[u8],
) -> Result<()> {
    if xattr_rule(name).is_some() {
        parse_policy_value(value)?;
        return Ok(());
    }
    if name.full_name() == MANAGED_LABEL_XATTR {
        parse_label_value(value)?;
        return Ok(());
    }

    Ok(())
}

pub(in crate::security) fn sync_aster_mac_inode_xattr(
    inode: &Arc<dyn Inode>,
    name: &XattrName<'_>,
    value: Option<&[u8]>,
) -> Result<()> {
    let state = inode_security_state(inode.as_ref());
    state.ensure_hydrated(inode.as_ref());

    if let Some(rule) = xattr_rule(name) {
        let enabled = match value {
            Some(value) => parse_policy_value(value)?,
            None => false,
        };
        state.update(rule, enabled);
        return Ok(());
    }
    if name.full_name() == MANAGED_LABEL_XATTR {
        let label = value.map(parse_label_value).transpose()?;
        state.set_label(label);
    }

    Ok(())
}

fn xattr_rule(name: &XattrName<'_>) -> Option<AsterMacPolicyRules> {
    match name.full_name() {
        "security.aster_mac.open" => Some(AsterMacPolicyRules::OPEN_DENY),
        "security.aster_mac.read" => Some(AsterMacPolicyRules::READ_DENY),
        "security.aster_mac.write" => Some(AsterMacPolicyRules::WRITE_DENY),
        "security.aster_mac.exec" => Some(AsterMacPolicyRules::EXEC_DENY),
        _ => None,
    }
}

fn load_inode_policy_rules(inode: &dyn Inode) -> AsterMacPolicyRules {
    let mut rules = AsterMacPolicyRules::empty();

    for (name, rule) in [
        ("security.aster_mac.open", AsterMacPolicyRules::OPEN_DENY),
        ("security.aster_mac.read", AsterMacPolicyRules::READ_DENY),
        ("security.aster_mac.write", AsterMacPolicyRules::WRITE_DENY),
        ("security.aster_mac.exec", AsterMacPolicyRules::EXEC_DENY),
    ] {
        let Some(value) = read_aster_mac_inode_xattr(inode, name) else {
            continue;
        };
        match parse_policy_value(&value) {
            Ok(true) => rules |= rule,
            Ok(false) => {}
            Err(err) => warn!(
                "ignore invalid Aster MAC inode policy xattr {} on inode {}: {:?}",
                name,
                inode.ino(),
                err.error()
            ),
        }
    }

    rules
}

fn load_inode_label(inode: &dyn Inode) -> Option<String> {
    let value = read_aster_mac_inode_xattr(inode, MANAGED_LABEL_XATTR)?;
    match parse_label_value(&value) {
        Ok(label) => Some(label),
        Err(err) => {
            warn!(
                "ignore invalid Aster MAC inode label on inode {}: {:?}",
                inode.ino(),
                err.error()
            );
            None
        }
    }
}

fn read_aster_mac_inode_xattr(inode: &dyn Inode, full_name: &'static str) -> Option<Vec<u8>> {
    let xattr_name = XattrName::try_from_full_name(full_name).unwrap();
    let mut value = vec![0u8; XATTR_VALUE_MAX_LEN.min(MAX_LABEL_LEN.max(16))];
    let mut writer = VmWriter::from(value.as_mut_slice()).to_fallible();

    match inode.get_xattr_without_permission_check(xattr_name, &mut writer) {
        Ok(len) => {
            value.truncate(len);
            Some(value)
        }
        Err(err)
            if err.error() == Errno::ENODATA
                || err.error() == Errno::EOPNOTSUPP
                || err.error() == Errno::EPERM =>
        {
            None
        }
        Err(err) => {
            warn!(
                "failed to read Aster MAC inode xattr {} from inode {}: {:?}",
                full_name,
                inode.ino(),
                err.error()
            );
            None
        }
    }
}

fn parse_policy_value(value: &[u8]) -> Result<bool> {
    let value = core::str::from_utf8(value)
        .map_err(|_| Error::with_message(Errno::EINVAL, "the xattr value is not valid UTF-8"))?;
    let value = value.trim_matches(|ch: char| ch.is_ascii_whitespace() || ch == '\0');

    match value {
        "1" | "true" | "deny" => Ok(true),
        "" | "0" | "false" | "allow" => Ok(false),
        _ => return_errno_with_message!(
            Errno::EINVAL,
            "the xattr value must be one of 0, 1, false, true, allow, or deny"
        ),
    }
}

fn parse_label_value(value: &[u8]) -> Result<String> {
    let label = core::str::from_utf8(value)
        .map_err(|_| Error::with_message(Errno::EINVAL, "the label is not valid UTF-8"))?;
    let label = label.trim_matches(|ch: char| ch.is_ascii_whitespace() || ch == '\0');

    if label.is_empty() {
        return_errno_with_message!(Errno::EINVAL, "the label cannot be empty");
    }
    if label.len() > MAX_LABEL_LEN {
        return_errno_with_message!(Errno::EINVAL, "the label is too long");
    }
    if !label
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | ':' | '/'))
    {
        return_errno_with_message!(Errno::EINVAL, "the label contains unsupported characters");
    }

    Ok(label.to_string())
}
