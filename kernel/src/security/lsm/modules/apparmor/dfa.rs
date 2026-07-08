// SPDX-License-Identifier: MPL-2.0

use super::{
    path::{AppArmorExecMode, AppArmorExecTransition, AppArmorFilePermission, AppArmorPathView},
    profile::AppArmorProfileName,
};
use crate::prelude::*;

const YYTH_MAGIC: u32 = 0x1b5e_783d;
const YYTH_FLAG_DIFF_ENCODE: u16 = 1;
const YYTH_FLAG_OOB_TRANS: u16 = 2;
const YYTH_FLAGS: u16 = YYTH_FLAG_DIFF_ENCODE | YYTH_FLAG_OOB_TRANS;
const YYTD_ID_ACCEPT: usize = 0;
const YYTD_ID_BASE: usize = 1;
const YYTD_ID_CHK: usize = 2;
const YYTD_ID_DEF: usize = 3;
const YYTD_ID_EC: usize = 4;
const YYTD_ID_ACCEPT2: usize = 6;
const YYTD_ID_NXT: usize = 7;
const YYTD_ID_TSIZE: usize = 8;
const YYTD_ID_MAX: usize = 8;
const YYTD_DATA8: u16 = 1;
const YYTD_DATA16: u16 = 2;
const YYTD_DATA32: u16 = 4;
const DFA_NOMATCH: u32 = 0;
const MATCH_FLAG_DIFF_ENCODE: u32 = 0x8000_0000;
const MATCH_FLAG_OOB_TRANSITION: u32 = 0x2000_0000;
const MATCH_FLAGS_MASK: u32 = 0xff00_0000;
const MATCH_FLAGS_VALID: u32 = MATCH_FLAG_DIFF_ENCODE | MATCH_FLAG_OOB_TRANSITION;
const MATCH_FLAGS_INVALID: u32 = MATCH_FLAGS_MASK & !MATCH_FLAGS_VALID;
const BASE_INDEX_MASK: u32 = 0x00ff_ffff;
const AA_X_INDEX_MASK: u32 = 0x00ff_ffff;
const AA_X_TYPE_MASK: u32 = 0x0c00_0000;
const AA_X_NAME: u32 = 0x0400_0000;
const AA_X_TABLE: u32 = 0x0800_0000;
const AA_X_INHERIT: u32 = 0x4000_0000;
const AA_X_UNCONFINED: u32 = 0x8000_0000;

/// A DFA-backed file policy decoded from Linux AppArmor binary policy.
#[derive(Clone, Debug)]
pub struct AppArmorDfaFilePolicy {
    dfa: AppArmorDfa,
    start: u32,
    permissions: Vec<AppArmorDfaPermissions>,
    transitions: Vec<AppArmorExecTransition>,
}

impl AppArmorDfaFilePolicy {
    /// Creates a DFA-backed file policy.
    pub(super) fn new(
        dfa: AppArmorDfa,
        start: u32,
        permissions: Vec<AppArmorDfaPermissions>,
        transitions: Vec<AppArmorExecTransition>,
    ) -> Result<Self> {
        dfa.verify_accept_indexes(permissions.len())?;
        if start as usize >= dfa.state_count() {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor DFA start state is invalid");
        }

        Ok(Self {
            dfa,
            start,
            permissions,
            transitions,
        })
    }

    /// Creates a DFA-backed file policy from the legacy inline permission format.
    pub(super) fn new_legacy(
        mut dfa: AppArmorDfa,
        start: u32,
        transitions: Vec<AppArmorExecTransition>,
    ) -> Result<Self> {
        let permissions = dfa.legacy_permissions()?;
        for (index, accept) in dfa.accept.iter_mut().enumerate() {
            *accept = u32::try_from(index)
                .map_err(|_| Error::with_message(Errno::EINVAL, "too many AppArmor DFA states"))?;
        }

        Self::new(dfa, start, permissions, transitions)
    }

    /// Evaluates access to a path.
    pub fn evaluate_path_access(
        &self,
        path_view: &AppArmorPathView,
        requested_permissions: AppArmorFilePermission,
    ) -> Result<AppArmorDfaAccessOutcome> {
        let requested_bits = requested_permissions.to_linux_bits();
        if requested_bits == 0 {
            return Ok(AppArmorDfaAccessOutcome::allow());
        }

        let state = self
            .dfa
            .match_bytes(self.start, path_view.as_str().as_bytes());
        let Some(permission_index) = self.dfa.accept_index(state) else {
            return Ok(AppArmorDfaAccessOutcome::deny(requested_permissions));
        };
        let Some(permissions) = self.permissions.get(permission_index) else {
            return_errno_with_message!(
                Errno::EINVAL,
                "the AppArmor DFA permission index is invalid"
            );
        };

        let explicitly_denied = requested_bits & permissions.deny;
        let missing = requested_bits & !permissions.allow;
        let denied_bits = explicitly_denied | missing;
        let denied = AppArmorFilePermission::from_linux_bits(denied_bits);
        let exec_transition = self.exec_transition(permissions, path_view)?;

        Ok(AppArmorDfaAccessOutcome {
            denied,
            exec_transition,
            audit: requested_bits & permissions.audit != 0,
        })
    }

    /// Returns whether this policy accepts `path_view` as an attachment.
    pub(super) fn matches_path(&self, path_view: &AppArmorPathView) -> bool {
        let state = self
            .dfa
            .match_bytes(self.start, path_view.as_str().as_bytes());
        if state == DFA_NOMATCH {
            return false;
        }

        let Some(permission_index) = self.dfa.accept_index(state) else {
            return false;
        };
        self.permissions
            .get(permission_index)
            .is_some_and(|permissions| permissions.allow != 0 && permissions.deny == 0)
    }

    fn exec_transition(
        &self,
        permissions: &AppArmorDfaPermissions,
        path_view: &AppArmorPathView,
    ) -> Result<AppArmorExecTransition> {
        let xindex = permissions.xindex;
        if xindex & AA_X_INHERIT != 0 {
            return Ok(AppArmorExecTransition::Inherit);
        }
        if xindex & AA_X_UNCONFINED != 0 {
            return Ok(AppArmorExecTransition::unconfined(AppArmorExecMode::Unsafe));
        }

        match xindex & AA_X_TYPE_MASK {
            AA_X_NAME => Ok(AppArmorExecTransition::profile(
                AppArmorProfileName::new(path_view.as_str().to_string())?,
                AppArmorExecMode::Unsafe,
            )),
            AA_X_TABLE => {
                let index = (xindex & AA_X_INDEX_MASK) as usize;
                let Some(transition) = self.transitions.get(index) else {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "the AppArmor exec transition index is invalid"
                    );
                };
                Ok(transition.clone())
            }
            _ => Ok(AppArmorExecTransition::Inherit),
        }
    }
}

/// A path-access decision from a DFA-backed policy.
pub struct AppArmorDfaAccessOutcome {
    /// Permissions denied by the policy.
    pub denied: AppArmorFilePermission,
    /// Executable profile transition selected by the matching state.
    pub exec_transition: AppArmorExecTransition,
    /// Whether matching permissions requested auditing.
    pub audit: bool,
}

impl AppArmorDfaAccessOutcome {
    fn allow() -> Self {
        Self {
            denied: AppArmorFilePermission::empty(),
            exec_transition: AppArmorExecTransition::Inherit,
            audit: false,
        }
    }

    fn deny(permissions: AppArmorFilePermission) -> Self {
        Self {
            denied: permissions,
            exec_transition: AppArmorExecTransition::Inherit,
            audit: false,
        }
    }
}

/// Permissions stored in a Linux AppArmor permstable entry.
#[derive(Clone, Debug, Default)]
pub(super) struct AppArmorDfaPermissions {
    pub allow: u32,
    pub deny: u32,
    pub audit: u32,
    pub xindex: u32,
}

impl AppArmorDfaPermissions {
    /// Creates a permission entry from a Linux AppArmor permstable row.
    pub(super) fn new(allow: u32, deny: u32, audit: u32, xindex: u32) -> Self {
        Self {
            allow,
            deny,
            audit,
            xindex,
        }
    }
}

/// A Linux AppArmor serialized DFA.
#[derive(Clone, Debug)]
pub(super) struct AppArmorDfa {
    flags: u16,
    accept: Vec<u32>,
    accept2: Option<Vec<u32>>,
    base: Vec<u32>,
    default: Vec<u32>,
    next: Vec<u32>,
    check: Vec<u32>,
    equivalence: Option<Vec<u8>>,
}

impl AppArmorDfa {
    /// Unpacks a Linux AppArmor DFA blob.
    pub(super) fn unpack(blob: &[u8]) -> Result<Self> {
        let mut reader = DfaReader::new(blob);
        let magic = reader.read_be_u32()?;
        if magic != YYTH_MAGIC {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor DFA magic is invalid");
        }

        let header_size = reader.read_be_u32()? as usize;
        let _serialized_size = reader.read_be_u32()?;
        let flags = reader.read_be_u16()?;
        if flags & !YYTH_FLAGS != 0 {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor DFA flags are invalid");
        }
        if header_size < reader.offset() || header_size > blob.len() {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor DFA header is invalid");
        }
        reader.skip_to(header_size)?;

        let mut tables = DfaTables::default();
        while reader.has_remaining() {
            let table = reader.read_table()?;
            tables.insert(table)?;
        }

        let dfa = tables.into_dfa(flags)?;
        dfa.verify_tables()?;
        Ok(dfa)
    }

    fn state_count(&self) -> usize {
        self.base.len()
    }

    fn accept_index(&self, state: u32) -> Option<usize> {
        self.accept
            .get(state as usize)
            .copied()
            .map(|index| index as usize)
    }

    fn match_bytes(&self, start: u32, bytes: &[u8]) -> u32 {
        if start == DFA_NOMATCH {
            return DFA_NOMATCH;
        }

        let mut state = start;
        for byte in bytes {
            let transition = self
                .equivalence
                .as_ref()
                .map(|equivalence| equivalence[usize::from(*byte)])
                .unwrap_or(*byte);
            state = self.next_state(state, u32::from(transition));
        }

        state
    }

    fn next_state(&self, mut state: u32, transition: u32) -> u32 {
        loop {
            let Some(base) = self.base.get(state as usize).copied() else {
                return DFA_NOMATCH;
            };
            let Some(index) = base_index(base).checked_add(transition) else {
                return DFA_NOMATCH;
            };
            let index = index as usize;
            if self.check.get(index).copied() == Some(state) {
                return self.next.get(index).copied().unwrap_or(DFA_NOMATCH);
            }

            state = self
                .default
                .get(state as usize)
                .copied()
                .unwrap_or(DFA_NOMATCH);
            if base & MATCH_FLAG_DIFF_ENCODE == 0 {
                return state;
            }
        }
    }

    fn verify_tables(&self) -> Result<()> {
        let state_count = self.state_count();
        if state_count < 2 || self.default.len() != state_count || self.accept.len() != state_count
        {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor DFA state tables are invalid");
        }
        if let Some(accept2) = &self.accept2
            && accept2.len() != state_count
        {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor DFA accept2 table is invalid");
        }
        if self.next.len() != self.check.len() {
            return_errno_with_message!(
                Errno::EINVAL,
                "the AppArmor DFA transition tables are invalid"
            );
        }
        if let Some(equivalence) = &self.equivalence
            && equivalence.len() != 256
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "the AppArmor DFA equivalence table is invalid"
            );
        }

        let transition_count = self.next.len() as u32;
        for (index, base) in self.base.iter().copied().enumerate() {
            if self.default[index] as usize >= state_count {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the AppArmor DFA default state is out of bounds"
                );
            }
            if base & MATCH_FLAGS_INVALID != 0 {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the AppArmor DFA state flags are invalid"
                );
            }
            if base & MATCH_FLAG_OOB_TRANSITION != 0 {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "AppArmor DFA out-of-band transitions are not supported"
                );
            }
            let Some(upper_bound) = base_index(base).checked_add(255) else {
                return_errno_with_message!(Errno::EINVAL, "the AppArmor DFA base index overflows");
            };
            if upper_bound >= transition_count {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the AppArmor DFA base index is out of bounds"
                );
            }
        }

        for next_state in &self.next {
            if *next_state as usize >= state_count {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the AppArmor DFA next state is out of bounds"
                );
            }
        }
        for check_state in &self.check {
            if *check_state as usize >= state_count {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the AppArmor DFA check state is out of bounds"
                );
            }
        }

        let _flags = self.flags;
        Ok(())
    }

    fn verify_accept_indexes(&self, permission_count: usize) -> Result<()> {
        for index in &self.accept {
            if *index as usize >= permission_count {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the AppArmor DFA accept index is out of bounds"
                );
            }
        }

        Ok(())
    }

    fn legacy_permissions(&self) -> Result<Vec<AppArmorDfaPermissions>> {
        let mut permissions = Vec::with_capacity(self.accept.len());
        for (state, accept) in self.accept.iter().copied().enumerate() {
            let accept2 = self
                .accept2
                .as_ref()
                .and_then(|table| table.get(state))
                .copied()
                .unwrap_or(0);
            permissions.push(AppArmorDfaPermissions::new(
                map_legacy_file_permissions_to_linux_bits(legacy_user_allow(accept)),
                0,
                map_legacy_file_permissions_to_linux_bits(legacy_user_audit(accept2)),
                map_legacy_xindex(accept & OLD_PERM_EXEC_MASK),
            ));
        }

        Ok(permissions)
    }
}

fn base_index(base: u32) -> u32 {
    base & BASE_INDEX_MASK
}

const OLD_PERM_EXEC_MASK: u32 = 0x3fff;
const OLD_PERM_USER_MASK: u32 = 0x7f;
const OLD_PERM_MAY_EXEC: u32 = 0x01;
const OLD_PERM_MAY_WRITE: u32 = 0x02;
const OLD_PERM_MAY_READ: u32 = 0x04;
const OLD_PERM_MAY_APPEND: u32 = 0x08;
const OLD_PERM_MAY_LINK: u32 = 0x10;
const OLD_PERM_EXEC_MMAP: u32 = 0x40;
const LEGACY_PERM_MAY_MMAP_EXEC: u32 = 0x0001_0000;
const OLD_X_UNCONFINED: u32 = 0x80;
const OLD_X_UNSAFE: u32 = 0x100;
const OLD_X_INHERIT: u32 = 0x200;
const LINUX_PERM_EXECUTE: u32 = 1 << 0;
const LINUX_PERM_WRITE: u32 = 1 << 1;
const LINUX_PERM_READ: u32 = 1 << 2;
const LINUX_PERM_APPEND: u32 = 1 << 3;
const LINUX_PERM_CREATE: u32 = 0x0010;
const LINUX_PERM_DELETE: u32 = 0x0020;
const LINUX_PERM_SETATTR: u32 = 0x0100;
const LINUX_PERM_MMAP: u32 = 0x0001_0000;
const LINUX_PERM_LINK: u32 = 0x0004_0000;

fn legacy_user_allow(accept: u32) -> u32 {
    accept & (OLD_PERM_USER_MASK | LEGACY_PERM_MAY_MMAP_EXEC)
}

fn legacy_user_audit(accept2: u32) -> u32 {
    accept2 & (OLD_PERM_USER_MASK | LEGACY_PERM_MAY_MMAP_EXEC)
}

fn map_legacy_file_permissions_to_linux_bits(old_permissions: u32) -> u32 {
    let mut permissions = 0;

    if old_permissions & OLD_PERM_MAY_EXEC != 0 {
        permissions |= LINUX_PERM_EXECUTE;
    }
    if old_permissions & OLD_PERM_MAY_WRITE != 0 {
        permissions |=
            LINUX_PERM_WRITE | LINUX_PERM_CREATE | LINUX_PERM_DELETE | LINUX_PERM_SETATTR;
    }
    if old_permissions & OLD_PERM_MAY_READ != 0 {
        permissions |= LINUX_PERM_READ;
    }
    if old_permissions & OLD_PERM_MAY_APPEND != 0 {
        permissions |= LINUX_PERM_APPEND;
    }
    if old_permissions & OLD_PERM_MAY_LINK != 0 {
        permissions |= LINUX_PERM_LINK;
    }
    if old_permissions & (OLD_PERM_EXEC_MMAP | LEGACY_PERM_MAY_MMAP_EXEC) != 0 {
        permissions |= LINUX_PERM_MMAP;
    }

    permissions
}

fn map_legacy_xindex(mask: u32) -> u32 {
    let old_index = (mask >> 10) & 0xf;
    let mut xindex = 0;

    if mask & OLD_X_INHERIT != 0 {
        xindex |= AA_X_INHERIT;
    }
    if mask & OLD_X_UNCONFINED != 0 || old_index == 1 {
        xindex |= AA_X_UNCONFINED;
    } else if old_index == 2 || old_index == 3 {
        xindex |= AA_X_NAME;
    } else if old_index != 0 {
        xindex |= AA_X_TABLE | (old_index - 4);
    }

    let _unsafe_exec = mask & OLD_X_UNSAFE;
    xindex
}

#[derive(Default)]
struct DfaTables {
    tables: [Option<DfaTable>; YYTD_ID_TSIZE],
}

impl DfaTables {
    fn insert(&mut self, table: DfaTable) -> Result<()> {
        let id = table.id;
        if id > YYTD_ID_MAX || id == YYTD_ID_TSIZE {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor DFA table id is invalid");
        }
        if self.tables[id].is_some() {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor DFA table is duplicated");
        }
        self.tables[id] = Some(table);
        Ok(())
    }

    fn into_dfa(mut self, flags: u16) -> Result<AppArmorDfa> {
        let accept = self.take_u32_table(YYTD_ID_ACCEPT)?;
        let base = self.take_u32_table(YYTD_ID_BASE)?;
        let default = self.take_u32_table(YYTD_ID_DEF)?;
        let next = self.take_u32_table(YYTD_ID_NXT)?;
        let check = self.take_u32_table(YYTD_ID_CHK)?;
        let equivalence = self.take_u8_table(YYTD_ID_EC)?;
        let accept2 = if self.tables[YYTD_ID_ACCEPT2].is_some() {
            Some(self.take_u32_table(YYTD_ID_ACCEPT2)?)
        } else {
            None
        };

        Ok(AppArmorDfa {
            flags,
            accept,
            accept2,
            base,
            default,
            next,
            check,
            equivalence,
        })
    }

    fn take_u8_table(&mut self, id: usize) -> Result<Option<Vec<u8>>> {
        let Some(table) = self.tables[id].take() else {
            return Ok(None);
        };
        table.into_u8_table()
    }

    fn take_u32_table(&mut self, id: usize) -> Result<Vec<u32>> {
        let Some(table) = self.tables[id].take() else {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor DFA table is missing");
        };
        table.into_u32_table()
    }
}

#[derive(Debug)]
struct DfaTable {
    id: usize,
    data: DfaTableData,
}

impl DfaTable {
    fn into_u8_table(self) -> Result<Option<Vec<u8>>> {
        match self.data {
            DfaTableData::U8(data) => Ok(Some(data)),
            _ => {
                return_errno_with_message!(Errno::EINVAL, "the AppArmor DFA table width is invalid")
            }
        }
    }

    fn into_u32_table(self) -> Result<Vec<u32>> {
        match self.data {
            DfaTableData::U16(data) => Ok(data.into_iter().map(u32::from).collect()),
            DfaTableData::U32(data) => Ok(data),
            _ => {
                return_errno_with_message!(Errno::EINVAL, "the AppArmor DFA table width is invalid")
            }
        }
    }
}

#[derive(Debug)]
enum DfaTableData {
    U8(Vec<u8>),
    U16(Vec<u16>),
    U32(Vec<u32>),
}

struct DfaReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> DfaReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn has_remaining(&self) -> bool {
        self.offset < self.bytes.len()
    }

    fn offset(&self) -> usize {
        self.offset
    }

    fn skip_to(&mut self, offset: usize) -> Result<()> {
        if offset > self.bytes.len() {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor DFA is truncated");
        }
        self.offset = offset;
        Ok(())
    }

    fn read_table(&mut self) -> Result<DfaTable> {
        let table_start = self.offset;
        let id = usize::from(self.read_be_u16()?);
        let flags = self.read_be_u16()?;
        let _high_len = self.read_be_u32()?;
        let low_len = self.read_be_u32()? as usize;
        if id == 0 {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor DFA table id is invalid");
        }
        let id = id - 1;

        let data = match flags {
            YYTD_DATA8 => DfaTableData::U8(self.read_be_u8_array(low_len)?),
            YYTD_DATA16 => DfaTableData::U16(self.read_be_u16_array(low_len)?),
            YYTD_DATA32 => DfaTableData::U32(self.read_be_u32_array(low_len)?),
            _ => return_errno_with_message!(
                Errno::EINVAL,
                "the AppArmor DFA table flags are invalid"
            ),
        };
        let table_size = align_up(self.offset - table_start, 8)?;
        self.skip_to(table_start + table_size)?;

        Ok(DfaTable { id, data })
    }

    fn read_be_u8_array(&mut self, len: usize) -> Result<Vec<u8>> {
        Ok(self.read_bytes(len)?.to_vec())
    }

    fn read_be_u16_array(&mut self, len: usize) -> Result<Vec<u16>> {
        let mut array = Vec::with_capacity(len);
        for _ in 0..len {
            array.push(self.read_be_u16()?);
        }
        Ok(array)
    }

    fn read_be_u32_array(&mut self, len: usize) -> Result<Vec<u32>> {
        let mut array = Vec::with_capacity(len);
        for _ in 0..len {
            array.push(self.read_be_u32()?);
        }
        Ok(array)
    }

    fn read_be_u16(&mut self) -> Result<u16> {
        let bytes = self.read_bytes(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn read_be_u32(&mut self) -> Result<u32> {
        let bytes = self.read_bytes(4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        let Some(end) = self.offset.checked_add(len) else {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor DFA is truncated");
        };
        if end > self.bytes.len() {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor DFA is truncated");
        }

        let bytes = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(bytes)
    }
}

fn align_up(value: usize, align: usize) -> Result<usize> {
    let Some(value) = value.checked_add(align - 1) else {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor DFA size overflows");
    };
    Ok(value & !(align - 1))
}
