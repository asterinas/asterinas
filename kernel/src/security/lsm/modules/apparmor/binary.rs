// SPDX-License-Identifier: MPL-2.0

use core::str;

use super::{
    attachment::AppArmorAttachment,
    capability::AppArmorCapabilityPolicy,
    dfa::{AppArmorDfa, AppArmorDfaFilePolicy, AppArmorDfaPermissions},
    path::{
        AppArmorExecMode, AppArmorExecTransition, AppArmorFilePermission, AppArmorPathPattern,
        AppArmorPathRule,
    },
    policy_update::AppArmorPolicyUpdate,
    profile::{AppArmorFilePolicy, AppArmorProfile, AppArmorProfileName},
    state::AppArmorMode,
};
use crate::{prelude::*, process::credentials::capabilities::CapSet};

const ASTERINAS_MAGIC: &[u8; 8] = b"AASTAA01";
const ASTERINAS_OP_REPLACE: u8 = 1;
const ASTERINAS_OP_REMOVE: u8 = 2;
const ASTERINAS_MODE_NONE: u8 = 0;
const ASTERINAS_MODE_ENFORCE: u8 = 1;
const ASTERINAS_MODE_COMPLAIN: u8 = 2;
const ASTERINAS_TRANSITION_INHERIT: u8 = 0;
const ASTERINAS_TRANSITION_UNCONFINED: u8 = 1;
const ASTERINAS_TRANSITION_PROFILE: u8 = 2;
const ASTERINAS_RULE_DENY: u8 = 1 << 0;
const ASTERINAS_RULE_AUDIT: u8 = 1 << 1;
const LINUX_MIN_ABI_VERSION: u32 = 5;
const LINUX_MAX_ABI_VERSION: u32 = 9;
const LINUX_FORCE_COMPLAIN_FLAG: u32 = 1 << 11;
const LINUX_KERNEL_ABI_MASK: u32 = 0x3ff;
const LINUX_PACKED_MODE_ENFORCE: u32 = 0;
const LINUX_PACKED_MODE_COMPLAIN: u32 = 1;
const LINUX_PACKED_MODE_KILL: u32 = 2;
const LINUX_PACKED_MODE_UNCONFINED: u32 = 3;
const LINUX_PACKED_MODE_USER: u32 = 4;
const LINUX_PACKED_FLAG_HAT: u32 = 1;

/// The expected policy operation for a securityfs policy control file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppArmorPolicyOperation {
    /// Inserts or replaces a profile.
    Replace,
    /// Removes a profile.
    Remove,
}

impl AppArmorPolicyOperation {
    fn from_asterinas_opcode(opcode: u8) -> Result<Self> {
        match opcode {
            ASTERINAS_OP_REPLACE => Ok(Self::Replace),
            ASTERINAS_OP_REMOVE => Ok(Self::Remove),
            _ => return_errno_with_message!(Errno::EINVAL, "the AppArmor policy opcode is invalid"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LinuxCode {
    U8 = 0,
    U16 = 1,
    U32 = 2,
    U64 = 3,
    Name = 4,
    String = 5,
    Blob = 6,
    Struct = 7,
    StructEnd = 8,
    Array = 11,
    ArrayEnd = 12,
}

impl LinuxCode {
    fn from_byte(byte: u8) -> Result<Self> {
        match byte {
            0 => Ok(Self::U8),
            1 => Ok(Self::U16),
            2 => Ok(Self::U32),
            3 => Ok(Self::U64),
            4 => Ok(Self::Name),
            5 => Ok(Self::String),
            6 => Ok(Self::Blob),
            7 => Ok(Self::Struct),
            8 => Ok(Self::StructEnd),
            11 => Ok(Self::Array),
            12 => Ok(Self::ArrayEnd),
            _ => return_errno_with_message!(
                Errno::EINVAL,
                "the AppArmor policy type code is invalid"
            ),
        }
    }
}

/// Returns whether a byte slice starts with a legacy Asterinas AppArmor magic.
pub fn has_binary_policy_magic(policy: &[u8]) -> bool {
    policy.starts_with(ASTERINAS_MAGIC)
}

pub(super) fn unpack_binary_policy(
    policy: &[u8],
    expected_operation: AppArmorPolicyOperation,
) -> Result<AppArmorPolicyUpdate> {
    if policy.starts_with(ASTERINAS_MAGIC) {
        unpack_asterinas_binary_policy(policy, expected_operation)
    } else {
        unpack_linux_binary_policy(policy, expected_operation)
    }
}

fn unpack_asterinas_binary_policy(
    policy: &[u8],
    expected_operation: AppArmorPolicyOperation,
) -> Result<AppArmorPolicyUpdate> {
    let mut reader = BinaryReader::new(policy);
    let magic = reader.read_bytes(ASTERINAS_MAGIC.len())?;
    if magic != ASTERINAS_MAGIC {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor binary policy magic is invalid");
    }

    let operation = AppArmorPolicyOperation::from_asterinas_opcode(reader.read_u8()?)?;
    if operation != expected_operation {
        return_errno_with_message!(
            Errno::EINVAL,
            "the AppArmor binary policy operation does not match the target file"
        );
    }

    let mode = parse_asterinas_mode(reader.read_u8()?)?;
    let _reserved = reader.read_u16()?;
    let name_len = usize::from(reader.read_u16()?);
    let rule_count = usize::from(reader.read_u16()?);
    let profile_name = read_profile_name(&mut reader, name_len)?;

    if profile_name.is_unconfined() {
        return_errno_with_message!(
            Errno::EINVAL,
            "the implicit unconfined AppArmor profile cannot be changed"
        );
    }

    let update = match operation {
        AppArmorPolicyOperation::Replace => {
            let Some(mode) = mode else {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "replacement AppArmor policies require a profile mode"
                );
            };
            let mut rules = Vec::with_capacity(rule_count);
            for _ in 0..rule_count {
                rules.push(read_asterinas_rule(&mut reader)?);
            }
            AppArmorPolicyUpdate::Replace(Box::new(AppArmorProfile::new(profile_name, mode, rules)))
        }
        AppArmorPolicyOperation::Remove => {
            if mode.is_some() || rule_count != 0 {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "remove AppArmor policies must not carry mode or rules"
                );
            }
            AppArmorPolicyUpdate::Remove(profile_name)
        }
    };

    if reader.has_remaining() {
        return_errno_with_message!(
            Errno::EINVAL,
            "the AppArmor binary policy has trailing bytes"
        );
    }

    Ok(update)
}

fn unpack_linux_binary_policy(
    policy: &[u8],
    expected_operation: AppArmorPolicyOperation,
) -> Result<AppArmorPolicyUpdate> {
    if expected_operation != AppArmorPolicyOperation::Replace {
        return_errno_with_message!(
            Errno::EINVAL,
            "Linux AppArmor remove operations are plain profile-name writes"
        );
    }

    let mut reader = LinuxPolicyReader::new(policy);
    let mut namespace: Option<String> = None;
    let mut profiles = Vec::new();
    while reader.has_remaining() {
        let header = reader.read_header(profiles.is_empty(), namespace.as_deref())?;
        if namespace.is_none() {
            namespace = header.namespace;
        }

        profiles.push(reader.read_profile(header.force_complain)?);
    }

    if profiles.is_empty() {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor policy has no profiles");
    }

    Ok(AppArmorPolicyUpdate::ReplaceMany(profiles))
}

fn parse_asterinas_mode(mode: u8) -> Result<Option<AppArmorMode>> {
    match mode {
        ASTERINAS_MODE_NONE => Ok(None),
        ASTERINAS_MODE_ENFORCE => Ok(Some(AppArmorMode::Enforce)),
        ASTERINAS_MODE_COMPLAIN => Ok(Some(AppArmorMode::Complain)),
        _ => return_errno_with_message!(Errno::EINVAL, "the AppArmor policy mode is invalid"),
    }
}

fn read_profile_name(reader: &mut BinaryReader<'_>, len: usize) -> Result<AppArmorProfileName> {
    AppArmorProfileName::new(read_string(reader, len)?)
}

fn read_asterinas_rule(reader: &mut BinaryReader<'_>) -> Result<AppArmorPathRule> {
    let flags = reader.read_u8()?;
    let transition = reader.read_u8()?;
    let permissions = read_permissions(reader.read_u32()?)?;
    let pattern_len = usize::from(reader.read_u16()?);
    let target_len = usize::from(reader.read_u16()?);
    let pattern = AppArmorPathPattern::new(read_string(reader, pattern_len)?);
    let target = read_string(reader, target_len)?;

    let deny = flags & ASTERINAS_RULE_DENY != 0;
    let audit = flags & ASTERINAS_RULE_AUDIT != 0;
    if flags & !(ASTERINAS_RULE_DENY | ASTERINAS_RULE_AUDIT) != 0 {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor rule flags are invalid");
    }

    let exec_transition = match transition {
        ASTERINAS_TRANSITION_INHERIT => {
            if !target.is_empty() {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "inherit AppArmor transitions must not name a target profile"
                );
            }
            AppArmorExecTransition::Inherit
        }
        ASTERINAS_TRANSITION_UNCONFINED => {
            if !target.is_empty() {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "unconfined AppArmor transitions must not name a target profile"
                );
            }
            AppArmorExecTransition::unconfined(AppArmorExecMode::Unsafe)
        }
        ASTERINAS_TRANSITION_PROFILE => AppArmorExecTransition::profile(
            AppArmorProfileName::new(target)?,
            AppArmorExecMode::Unsafe,
        ),
        _ => return_errno_with_message!(Errno::EINVAL, "the AppArmor exec transition is invalid"),
    };

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

fn read_permissions(bits: u32) -> Result<AppArmorFilePermission> {
    let Some(permissions) = AppArmorFilePermission::from_bits(bits) else {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor permissions are invalid");
    };
    if permissions.is_empty() {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor permissions are empty");
    }

    Ok(permissions)
}

fn read_capability_mask(low: u32, high: u32) -> Result<CapSet> {
    let bits = u64::from(low) | (u64::from(high) << 32);
    CapSet::try_from(bits).map_err(|_| {
        Error::with_message(
            Errno::EINVAL,
            "the AppArmor capability mask contains unsupported capabilities",
        )
    })
}

fn read_string(reader: &mut BinaryReader<'_>, len: usize) -> Result<String> {
    let bytes = reader.read_bytes(len)?;
    let text = str::from_utf8(bytes)
        .map_err(|_| Error::with_message(Errno::EINVAL, "the AppArmor string is not UTF-8"))?;
    if text.bytes().any(|byte| byte == 0) {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor string contains a nul byte");
    }

    Ok(text.to_string())
}

struct LinuxHeader {
    namespace: Option<String>,
    force_complain: bool,
}

struct LinuxProfileFlags {
    mode: AppArmorMode,
}

struct LinuxPolicyReader<'a> {
    bytes: &'a [u8],
    offset: usize,
    abi_version: u32,
}

impl<'a> LinuxPolicyReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            offset: 0,
            abi_version: 0,
        }
    }

    fn has_remaining(&self) -> bool {
        self.offset < self.bytes.len()
    }

    fn read_header(
        &mut self,
        required: bool,
        previous_namespace: Option<&str>,
    ) -> Result<LinuxHeader> {
        let Some(version) = self.read_optional_u32("version")? else {
            if required {
                return_errno_with_message!(
                    Errno::EPROTONOSUPPORT,
                    "the AppArmor policy version is missing"
                );
            }
            return Ok(LinuxHeader {
                namespace: None,
                force_complain: false,
            });
        };

        let abi_version = decode_linux_abi_version(version);
        if !(LINUX_MIN_ABI_VERSION..=LINUX_MAX_ABI_VERSION).contains(&abi_version) {
            return_errno_with_message!(
                Errno::EPROTONOSUPPORT,
                "the AppArmor policy ABI version is unsupported"
            );
        }
        self.abi_version = abi_version;

        let namespace = self.read_optional_string("namespace")?;
        if let Some(namespace) = namespace.as_deref() {
            if namespace.is_empty() {
                return_errno_with_message!(Errno::EINVAL, "the AppArmor namespace is empty");
            }
            if namespace != "root" {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "only the root AppArmor policy namespace is supported"
                );
            }
        }
        if let (Some(previous), Some(namespace)) = (previous_namespace, namespace.as_deref())
            && previous != namespace
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "one AppArmor policy payload cannot change namespaces"
            );
        }

        Ok(LinuxHeader {
            namespace,
            force_complain: version & LINUX_FORCE_COMPLAIN_FLAG != 0,
        })
    }

    fn read_profile(&mut self, force_complain: bool) -> Result<AppArmorProfile> {
        self.expect_name_code(LinuxCode::Struct, Some("profile"))?;
        let profile_name = AppArmorProfileName::new(self.read_string(None)?)?;
        if profile_name.is_unconfined() {
            return_errno_with_message!(
                Errno::EINVAL,
                "the implicit unconfined AppArmor profile cannot be loaded"
            );
        }

        if self.read_optional_string("rename")?.is_some() {
            return_errno_with_message!(
                Errno::EINVAL,
                "renaming AppArmor profiles is not supported"
            );
        }
        let attach = self.read_optional_string("attach")?;
        let attach_policy = self.read_optional_attach_policy()?;
        let _disconnected = self.read_optional_string("disconnected")?;
        let _kill_signal = self.read_optional_u32("kill")?;

        let profile = self.read_profile_flags(force_complain)?;
        let _path_flags = self.read_optional_u32("path_flags")?;
        let capability_policy = self.read_capabilities()?;
        self.read_optional_xattrs()?;
        self.read_optional_rlimits()?;
        self.read_optional_secmark()?;
        let domain_policy = self.read_optional_domain_policy()?;
        let file_policy = self.read_file_policy()?;
        self.read_optional_data_table()?;
        self.expect_name_code(LinuxCode::StructEnd, None)?;

        let attachment = AppArmorAttachment::from_profile(
            &profile_name,
            attach,
            attach_policy.or(domain_policy),
        );

        Ok(AppArmorProfile::new_with_policies(
            profile_name,
            attachment,
            profile.mode,
            file_policy,
            capability_policy,
        ))
    }

    fn read_profile_flags(&mut self, force_complain: bool) -> Result<LinuxProfileFlags> {
        self.expect_name_code(LinuxCode::Struct, Some("flags"))?;
        let flags = self.read_u32(None)?;
        if flags & LINUX_PACKED_FLAG_HAT != 0 {
            return_errno_with_message!(Errno::EINVAL, "AppArmor hats are not supported");
        }
        let packed_mode = self.read_u32(None)?;
        let _audit = self.read_u32(None)?;
        self.expect_name_code(LinuxCode::StructEnd, None)?;

        let mode = if force_complain || packed_mode == LINUX_PACKED_MODE_COMPLAIN {
            AppArmorMode::Complain
        } else if packed_mode == LINUX_PACKED_MODE_ENFORCE {
            AppArmorMode::Enforce
        } else if packed_mode == LINUX_PACKED_MODE_KILL
            || packed_mode == LINUX_PACKED_MODE_UNCONFINED
            || packed_mode == LINUX_PACKED_MODE_USER
        {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor profile mode is not supported");
        } else {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor profile mode is invalid");
        };

        Ok(LinuxProfileFlags { mode })
    }

    fn read_capabilities(&mut self) -> Result<AppArmorCapabilityPolicy> {
        let low_allow = self.read_u32(None)?;
        let low_audit = self.read_u32(None)?;
        let low_quiet = self.read_u32(None)?;
        let low_reserved = self.read_u32(None)?;
        let mut high_words = [0; 4];
        if self.consume_name_code(LinuxCode::Struct, Some("caps64"))? {
            for word in &mut high_words {
                *word = self.read_u32(None)?;
            }
            self.expect_name_code(LinuxCode::StructEnd, None)?;
        }
        if self.consume_name_code(LinuxCode::Struct, Some("capsx"))? {
            let extended_low = self.read_u32(None)?;
            let extended_high = self.read_u32(None)?;
            if extended_low != 0 || extended_high != 0 {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "AppArmor capability mediation is not supported"
                );
            }
            self.expect_name_code(LinuxCode::StructEnd, None)?;
        }

        if low_reserved != 0 || high_words[3] != 0 {
            return_errno_with_message!(
                Errno::EINVAL,
                "AppArmor capability kill mode is not supported"
            );
        }

        Ok(AppArmorCapabilityPolicy::new(
            read_capability_mask(low_allow, high_words[0])?,
            read_capability_mask(low_audit, high_words[1])?,
            read_capability_mask(low_quiet, high_words[2])?,
        ))
    }

    fn read_optional_attach_policy(&mut self) -> Result<Option<AppArmorDfaFilePolicy>> {
        let position = self.offset;
        match self.read_policydb(false, false)? {
            Some(policy) => Ok(Some(policy)),
            None => {
                self.offset = position;
                Ok(None)
            }
        }
    }

    fn read_optional_domain_policy(&mut self) -> Result<Option<AppArmorDfaFilePolicy>> {
        if !self.consume_name_code(LinuxCode::Struct, Some("policydb"))? {
            return Ok(None);
        }

        // Linux emits a profile-domain policydb before the file mediation policy.
        let domain_policy = self.read_policydb(true, false)?;
        self.expect_name_code(LinuxCode::StructEnd, None)?;
        Ok(domain_policy)
    }

    fn read_file_policy(&mut self) -> Result<AppArmorFilePolicy> {
        let Some(policy) = self.read_policydb(false, true)? else {
            return Ok(AppArmorFilePolicy::PathRules(Vec::new()));
        };
        Ok(AppArmorFilePolicy::Dfa(Box::new(policy)))
    }

    fn read_policydb(
        &mut self,
        required_dfa: bool,
        required_transitions: bool,
    ) -> Result<Option<AppArmorDfaFilePolicy>> {
        self.skip_optional_tags()?;
        let permissions = self.read_optional_permissions_table()?;
        let _perms_version = self.read_optional_u32("permsv")?;
        let dfa = self.read_optional_dfa()?;
        let start = self
            .read_optional_u32("start")?
            .or(self.read_optional_u32("dfa_start")?)
            .unwrap_or(1);
        let transitions = self.read_optional_transition_table()?;

        if dfa.is_none() {
            if required_dfa {
                return_errno_with_message!(Errno::EINVAL, "the AppArmor policydb DFA is missing");
            }
            if required_transitions && !transitions.is_empty() {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "AppArmor transition tables require a DFA"
                );
            }
            return Ok(None);
        }

        let dfa = dfa.unwrap();
        match permissions {
            Some(permissions) => {
                AppArmorDfaFilePolicy::new(dfa, start, permissions, transitions).map(Some)
            }
            None => AppArmorDfaFilePolicy::new_legacy(dfa, start, transitions).map(Some),
        }
    }

    fn skip_optional_tags(&mut self) -> Result<()> {
        if !self.consume_name_code(LinuxCode::Struct, Some("tags"))? {
            return Ok(());
        }

        return_errno_with_message!(Errno::EINVAL, "AppArmor policy tags are not supported")
    }

    fn read_optional_permissions_table(&mut self) -> Result<Option<Vec<AppArmorDfaPermissions>>> {
        if !self.consume_name_code(LinuxCode::Struct, Some("perms"))? {
            return Ok(None);
        }

        let version = self.read_u32(Some("version"))?;
        if version != 1 {
            return_errno_with_message!(
                Errno::EINVAL,
                "the AppArmor permission table version is unsupported"
            );
        }
        let count = usize::from(self.read_array(None)?);
        let mut permissions = Vec::with_capacity(count);
        for _ in 0..count {
            let _reserved = self.read_u32(None)?;
            let allow = self.read_u32(None)?;
            let deny = self.read_u32(None)?;
            let _subtree = self.read_u32(None)?;
            let _condition = self.read_u32(None)?;
            let _kill = self.read_u32(None)?;
            let _complain = self.read_u32(None)?;
            let _prompt = self.read_u32(None)?;
            let audit = self.read_u32(None)?;
            let _quiet = self.read_u32(None)?;
            let _hide = self.read_u32(None)?;
            let xindex = self.read_u32(None)?;
            let tag = self.read_u32(None)?;
            let label = self.read_u32(None)?;
            if tag != 0 || label != 0 {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "AppArmor tagged permissions are not supported"
                );
            }
            permissions.push(AppArmorDfaPermissions::new(allow, deny, audit, xindex));
        }
        self.expect_name_code(LinuxCode::ArrayEnd, None)?;
        self.expect_name_code(LinuxCode::StructEnd, None)?;

        Ok(Some(permissions))
    }

    fn read_optional_dfa(&mut self) -> Result<Option<AppArmorDfa>> {
        let Some((blob, content_start, content_end)) = self.read_optional_blob("aadfa")? else {
            return Ok(None);
        };
        let dfa_start = aligned_blob_payload_offset(content_start, content_end)?;
        if dfa_start > blob.len() {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor DFA blob is invalid");
        }

        AppArmorDfa::unpack(&blob[dfa_start..]).map(Some)
    }

    fn read_optional_transition_table(&mut self) -> Result<Vec<AppArmorExecTransition>> {
        if !self.consume_name_code(LinuxCode::Struct, Some("xtable"))? {
            return Ok(Vec::new());
        }

        let count = usize::from(self.read_array(None)?);
        let mut transitions = Vec::with_capacity(count);
        for _ in 0..count {
            let target = self.read_string(None)?;
            transitions.push(parse_linux_transition_target(target)?);
        }
        self.expect_name_code(LinuxCode::ArrayEnd, None)?;
        self.expect_name_code(LinuxCode::StructEnd, None)?;
        Ok(transitions)
    }

    fn read_optional_xattrs(&mut self) -> Result<()> {
        if !self.consume_name_code(LinuxCode::Struct, Some("xattrs"))? {
            return Ok(());
        }

        return_errno_with_message!(
            Errno::EINVAL,
            "AppArmor xattr attachments are not supported"
        )
    }

    fn read_optional_rlimits(&mut self) -> Result<()> {
        if !self.consume_name_code(LinuxCode::Struct, Some("rlimits"))? {
            return Ok(());
        }

        return_errno_with_message!(Errno::EINVAL, "AppArmor rlimit mediation is not supported")
    }

    fn read_optional_secmark(&mut self) -> Result<()> {
        if !self.consume_name_code(LinuxCode::Struct, Some("secmark"))? {
            return Ok(());
        }

        return_errno_with_message!(Errno::EINVAL, "AppArmor secmark mediation is not supported")
    }

    fn read_optional_data_table(&mut self) -> Result<()> {
        if !self.consume_name_code(LinuxCode::Struct, Some("data"))? {
            return Ok(());
        }

        while let Some(_key) = self.read_optional_string_any_name()? {
            let _ = self.read_optional_blob_any_name()?;
        }
        self.expect_name_code(LinuxCode::StructEnd, None)
    }

    fn read_optional_string(&mut self, name: &str) -> Result<Option<String>> {
        if !self.consume_name_code(LinuxCode::String, Some(name))? {
            return Ok(None);
        }
        self.read_string_payload().map(Some)
    }

    fn read_optional_string_any_name(&mut self) -> Result<Option<String>> {
        if !self.consume_name_code(LinuxCode::String, None)? {
            return Ok(None);
        }
        self.read_string_payload().map(Some)
    }

    fn read_string(&mut self, name: Option<&str>) -> Result<String> {
        self.expect_name_code(LinuxCode::String, name)?;
        self.read_string_payload()
    }

    fn read_string_payload(&mut self) -> Result<String> {
        let len = usize::from(self.read_le_u16_raw()?);
        let bytes = self.read_bytes(len)?;
        if bytes.last().copied() != Some(0) {
            return_errno_with_message!(
                Errno::EINVAL,
                "the AppArmor policy string is not nul-terminated"
            );
        }
        let text = str::from_utf8(&bytes[..bytes.len() - 1]).map_err(|_| {
            Error::with_message(Errno::EINVAL, "the AppArmor policy string is not UTF-8")
        })?;
        if text.bytes().any(|byte| byte == 0) {
            return_errno_with_message!(
                Errno::EINVAL,
                "the AppArmor policy string contains an embedded nul byte"
            );
        }

        Ok(text.to_string())
    }

    fn read_optional_blob(&mut self, name: &str) -> Result<Option<(&'a [u8], usize, usize)>> {
        if !self.consume_name_code(LinuxCode::Blob, Some(name))? {
            return Ok(None);
        }
        self.read_blob_payload().map(Some)
    }

    fn read_optional_blob_any_name(&mut self) -> Result<Option<(&'a [u8], usize, usize)>> {
        if !self.consume_name_code(LinuxCode::Blob, None)? {
            return Ok(None);
        }
        self.read_blob_payload().map(Some)
    }

    fn read_blob_payload(&mut self) -> Result<(&'a [u8], usize, usize)> {
        let len = self.read_le_u32_raw()? as usize;
        let content_start = self.offset;
        let bytes = self.read_bytes(len)?;
        let content_end = self.offset;
        Ok((bytes, content_start, content_end))
    }

    fn read_optional_u32(&mut self, name: &str) -> Result<Option<u32>> {
        if !self.consume_name_code(LinuxCode::U32, Some(name))? {
            return Ok(None);
        }
        self.read_le_u32_raw().map(Some)
    }

    fn read_u32(&mut self, name: Option<&str>) -> Result<u32> {
        self.expect_name_code(LinuxCode::U32, name)?;
        self.read_le_u32_raw()
    }

    fn read_array(&mut self, name: Option<&str>) -> Result<u16> {
        self.expect_name_code(LinuxCode::Array, name)?;
        self.read_le_u16_raw()
    }

    fn consume_name_code(&mut self, code: LinuxCode, name: Option<&str>) -> Result<bool> {
        let position = self.offset;
        match self.expect_name_code(code, name) {
            Ok(()) => Ok(true),
            Err(_) => {
                self.offset = position;
                Ok(false)
            }
        }
    }

    fn expect_name_code(&mut self, code: LinuxCode, name: Option<&str>) -> Result<()> {
        let position = self.offset;
        if self.peek_code()? == Some(LinuxCode::Name) {
            self.offset += 1;
            let tag = self.read_tag_name()?;
            if let Some(expected_name) = name
                && tag != expected_name
            {
                self.offset = position;
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the AppArmor policy field name is invalid"
                );
            }
        } else if name.is_some() {
            self.offset = position;
            return_errno_with_message!(Errno::EINVAL, "the AppArmor policy field name is missing");
        }

        let Some(actual_code) = self.peek_code()? else {
            self.offset = position;
            return_errno_with_message!(Errno::EINVAL, "the AppArmor policy is truncated");
        };
        if actual_code != code {
            self.offset = position;
            return_errno_with_message!(Errno::EINVAL, "the AppArmor policy field type is invalid");
        }
        self.offset += 1;
        Ok(())
    }

    fn read_tag_name(&mut self) -> Result<String> {
        let len = usize::from(self.read_le_u16_raw()?);
        let bytes = self.read_bytes(len)?;
        if bytes.last().copied() != Some(0) {
            return_errno_with_message!(
                Errno::EINVAL,
                "the AppArmor policy field name is not nul-terminated"
            );
        }
        let name = str::from_utf8(&bytes[..bytes.len() - 1]).map_err(|_| {
            Error::with_message(Errno::EINVAL, "the AppArmor field name is not UTF-8")
        })?;
        Ok(name.to_string())
    }

    fn peek_code(&self) -> Result<Option<LinuxCode>> {
        let Some(byte) = self.bytes.get(self.offset).copied() else {
            return Ok(None);
        };
        LinuxCode::from_byte(byte).map(Some)
    }

    fn read_le_u16_raw(&mut self) -> Result<u16> {
        let bytes = self.read_bytes(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn read_le_u32_raw(&mut self) -> Result<u32> {
        let bytes = self.read_bytes(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        let Some(end) = self.offset.checked_add(len) else {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor binary policy is truncated");
        };
        if end > self.bytes.len() {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor binary policy is truncated");
        }

        let bytes = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(bytes)
    }
}

fn decode_linux_abi_version(version: u32) -> u32 {
    if version <= LINUX_MIN_ABI_VERSION {
        version
    } else {
        version & LINUX_KERNEL_ABI_MASK
    }
}

fn aligned_blob_payload_offset(content_start: usize, content_end: usize) -> Result<usize> {
    let Some(unaligned) = content_start.checked_sub(content_end & 7) else {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor DFA blob is invalid");
    };
    let aligned = align_up(unaligned, 8)?;
    Ok(aligned - unaligned)
}

fn align_up(value: usize, align: usize) -> Result<usize> {
    let Some(value) = value.checked_add(align - 1) else {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor binary policy size overflows");
    };
    Ok(value & !(align - 1))
}

fn parse_linux_transition_target(target: String) -> Result<AppArmorExecTransition> {
    if target.is_empty() {
        return Ok(AppArmorExecTransition::Inherit);
    }
    if target == "unconfined" {
        return Ok(AppArmorExecTransition::unconfined(AppArmorExecMode::Unsafe));
    }

    AppArmorProfileName::new(target)
        .map(|profile_name| AppArmorExecTransition::profile(profile_name, AppArmorExecMode::Unsafe))
}

struct BinaryReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> BinaryReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn has_remaining(&self) -> bool {
        self.offset < self.bytes.len()
    }

    fn read_u8(&mut self) -> Result<u8> {
        let bytes = self.read_bytes(1)?;
        Ok(bytes[0])
    }

    fn read_u16(&mut self) -> Result<u16> {
        let bytes = self.read_bytes(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn read_u32(&mut self) -> Result<u32> {
        let bytes = self.read_bytes(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        let Some(end) = self.offset.checked_add(len) else {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor binary policy is truncated");
        };
        if end > self.bytes.len() {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor binary policy is truncated");
        }

        let bytes = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(bytes)
    }
}
