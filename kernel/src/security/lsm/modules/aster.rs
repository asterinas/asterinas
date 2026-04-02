// SPDX-License-Identifier: MPL-2.0

use alloc::boxed::ThinBox;
use core::sync::atomic::{AtomicU8, Ordering};

use super::super::{BprmCheckContext, FileOpenContext, InodePermissionContext, LsmKind, LsmModule};
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

pub(crate) static ASTER_LSM: AsterLsm = AsterLsm;

const MAX_POLICY_XATTR_LEN: usize = 16;

/// Implements a small xattr-driven policy module for file and exec hooks.
pub(crate) struct AsterLsm;

impl LsmModule for AsterLsm {
    fn name(&self) -> &'static str {
        "aster"
    }

    fn kind(&self) -> LsmKind {
        LsmKind::Minor
    }

    fn bprm_check_security(&self, context: &BprmCheckContext<'_>) -> Result<()> {
        let executable_inode = context.executable().inode();
        let rules =
            inode_security_state(executable_inode.as_ref()).rules(executable_inode.as_ref());
        if rules.contains(AsterPolicyRules::EXEC_DENY) {
            return_errno_with_message!(
                Errno::EACCES,
                "exec is denied by the Aster LSM xattr policy"
            );
        }

        Ok(())
    }

    fn inode_permission(&self, context: &InodePermissionContext<'_>) -> Result<()> {
        let inode = context.path().inode();
        let rules = inode_security_state(inode.as_ref()).rules(inode.as_ref());
        let permission = context.permission();

        if permission.may_read() && rules.contains(AsterPolicyRules::READ_DENY) {
            return_errno_with_message!(
                Errno::EACCES,
                "read access is denied by the Aster LSM xattr policy"
            );
        }
        if permission.may_write() && rules.contains(AsterPolicyRules::WRITE_DENY) {
            return_errno_with_message!(
                Errno::EACCES,
                "write access is denied by the Aster LSM xattr policy"
            );
        }
        if permission.may_exec() && rules.contains(AsterPolicyRules::EXEC_DENY) {
            return_errno_with_message!(
                Errno::EACCES,
                "exec access is denied by the Aster LSM xattr policy"
            );
        }

        Ok(())
    }

    fn file_open(&self, context: &FileOpenContext<'_>) -> Result<()> {
        let inode = context.path().inode();
        let rules = inode_security_state(inode.as_ref()).rules(inode.as_ref());
        let status_flags = context.status_flags();

        if rules.contains(AsterPolicyRules::OPEN_DENY) {
            return_errno_with_message!(
                Errno::EACCES,
                "open is denied by the Aster LSM xattr policy"
            );
        }
        if status_flags.contains(StatusFlags::O_PATH) {
            return Ok(());
        }
        if context.access_mode().is_readable() && rules.contains(AsterPolicyRules::READ_DENY) {
            return_errno_with_message!(
                Errno::EACCES,
                "read-open is denied by the Aster LSM xattr policy"
            );
        }
        if context.access_mode().is_writable() && rules.contains(AsterPolicyRules::WRITE_DENY) {
            return_errno_with_message!(
                Errno::EACCES,
                "write-open is denied by the Aster LSM xattr policy"
            );
        }

        Ok(())
    }
}

bitflags! {
    struct AsterPolicyRules: u8 {
        const OPEN_DENY  = 1 << 0;
        const READ_DENY  = 1 << 1;
        const WRITE_DENY = 1 << 2;
        const EXEC_DENY  = 1 << 3;
    }
}

struct AsterInodeSecurityState {
    rules: AtomicU8,
    is_hydrated: core::sync::atomic::AtomicBool,
}

impl AsterInodeSecurityState {
    fn new() -> Self {
        Self {
            rules: AtomicU8::new(0),
            is_hydrated: core::sync::atomic::AtomicBool::new(false),
        }
    }

    fn rules(&self, inode: &dyn Inode) -> AsterPolicyRules {
        self.ensure_hydrated(inode);
        AsterPolicyRules::from_bits_truncate(self.rules.load(Ordering::Relaxed))
    }

    fn update(&self, rule: AsterPolicyRules, enabled: bool) {
        if enabled {
            self.rules.fetch_or(rule.bits(), Ordering::Relaxed);
        } else {
            self.rules.fetch_and(!rule.bits(), Ordering::Relaxed);
        }
        self.is_hydrated
            .store(true, core::sync::atomic::Ordering::Relaxed);
    }

    fn ensure_hydrated(&self, inode: &dyn Inode) {
        if self.is_hydrated.load(core::sync::atomic::Ordering::Acquire) {
            return;
        }

        let rules = load_inode_policy_rules(inode);
        self.rules.store(rules.bits(), Ordering::Relaxed);
        self.is_hydrated
            .store(true, core::sync::atomic::Ordering::Release);
    }
}

fn inode_security_state(inode: &dyn Inode) -> &AsterInodeSecurityState {
    inode
        .extension()
        .group3()
        .call_once(|| ThinBox::new_unsize(AsterInodeSecurityState::new()))
        .downcast_ref()
        .unwrap()
}

pub(crate) fn is_aster_inode_xattr(name: &XattrName<'_>) -> bool {
    xattr_rule(name).is_some()
}

pub(crate) fn validate_aster_inode_xattr(name: &XattrName<'_>, value: &[u8]) -> Result<()> {
    if xattr_rule(name).is_some() {
        parse_policy_value(value)?;
    }

    Ok(())
}

pub(crate) fn sync_aster_inode_xattr(
    inode: &Arc<dyn Inode>,
    name: &XattrName<'_>,
    value: Option<&[u8]>,
) -> Result<()> {
    let Some(rule) = xattr_rule(name) else {
        return Ok(());
    };

    let enabled = match value {
        Some(value) => parse_policy_value(value)?,
        None => false,
    };

    let state = inode_security_state(inode.as_ref());
    state.ensure_hydrated(inode.as_ref());
    state.update(rule, enabled);

    Ok(())
}

fn xattr_rule(name: &XattrName<'_>) -> Option<AsterPolicyRules> {
    match name.full_name() {
        "security.aster.open" => Some(AsterPolicyRules::OPEN_DENY),
        "security.aster.read" => Some(AsterPolicyRules::READ_DENY),
        "security.aster.write" => Some(AsterPolicyRules::WRITE_DENY),
        "security.aster.exec" => Some(AsterPolicyRules::EXEC_DENY),
        _ => None,
    }
}

fn load_inode_policy_rules(inode: &dyn Inode) -> AsterPolicyRules {
    let mut rules = AsterPolicyRules::empty();

    for (name, rule) in [
        ("security.aster.open", AsterPolicyRules::OPEN_DENY),
        ("security.aster.read", AsterPolicyRules::READ_DENY),
        ("security.aster.write", AsterPolicyRules::WRITE_DENY),
        ("security.aster.exec", AsterPolicyRules::EXEC_DENY),
    ] {
        let Some(value) = read_aster_inode_xattr(inode, name) else {
            continue;
        };
        match parse_policy_value(&value) {
            Ok(true) => rules |= rule,
            Ok(false) => {}
            Err(err) => warn!(
                "ignore invalid Aster inode policy xattr {} on inode {}: {:?}",
                name,
                inode.ino(),
                err.error()
            ),
        }
    }

    rules
}

fn read_aster_inode_xattr(inode: &dyn Inode, full_name: &'static str) -> Option<Vec<u8>> {
    let xattr_name = XattrName::try_from_full_name(full_name).unwrap();
    let mut value = vec![0u8; XATTR_VALUE_MAX_LEN.min(MAX_POLICY_XATTR_LEN)];
    let mut writer = VmWriter::from(value.as_mut_slice()).to_fallible();

    match inode.get_xattr(xattr_name, &mut writer) {
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
                "failed to read Aster inode xattr {} from inode {}: {:?}",
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
