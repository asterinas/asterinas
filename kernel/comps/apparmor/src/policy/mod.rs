// SPDX-License-Identifier: MPL-2.0

//! AppArmor profiles and policy storage.
//!
//! A [`Profile`] contains validated metadata and the original policy bytes.
//! Only profiles with supported runtime rules can attach to tasks.
//! [`PolicyStore`] provides immutable [`PolicySet`] snapshots so readers can
//! finish a query while profiles are loaded or removed.

use alloc::{string::String, sync::Arc, vec::Vec};
use core::{fmt::Write, ops::Range};

use aster_core::prelude::*;
use aster_systree::{SysNodeId, SysStr};
use ostd::sync::Mutex;

mod abi;
mod dfa;

pub(super) const MAX_POLICY_BLOB_SIZE: usize = 1024 * 1024;
const MAX_LOADED_PROFILES: usize = 1024;
// Limit policy bytes in the current set; old snapshots and open nodes can retain more.
const MAX_POLICY_STORE_BYTES: usize = 16 * 1024 * 1024;
pub(super) const MAX_PROFILE_NAME_LEN: usize = 255;
const MAX_DIRECTORY_SUFFIX_LEN: usize = 21; // A dot and at most 20 decimal digits.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ProfileMode {
    Enforce,
    Complain,
    Kill,
    Unconfined,
    User,
}

impl ProfileMode {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Enforce => "enforce",
            Self::Complain => "complain",
            Self::Kill => "kill",
            Self::Unconfined => "unconfined",
            Self::User => "user",
        }
    }

    fn from_packed(raw: u32) -> Option<Self> {
        match raw {
            0 => Some(Self::Enforce),
            1 => Some(Self::Complain),
            2 => Some(Self::Kill),
            3 => Some(Self::Unconfined),
            4 => Some(Self::User),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub(super) struct Profile {
    name: String,
    mode: ProfileMode,
    raw_data: Vec<u8>,
    id: SysNodeId,
    directory_name: SysStr,
    runtime: Option<RuntimePolicy>,
}

#[derive(Debug)]
struct RuntimePolicy {
    attachment: Option<Attachment>,
}

#[derive(Debug)]
struct Attachment {
    range: Range<usize>,
    minimum_len: usize,
}

impl Profile {
    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn mode(&self) -> ProfileMode {
        self.mode
    }

    pub(super) fn raw_data(&self) -> &[u8] {
        &self.raw_data
    }

    pub(super) fn id(&self) -> &SysNodeId {
        &self.id
    }

    pub(super) fn directory_name(&self) -> &SysStr {
        &self.directory_name
    }

    fn new(
        name: String,
        mode: ProfileMode,
        raw_data: Vec<u8>,
        runtime: Option<RuntimePolicy>,
    ) -> Result<Self> {
        let id = SysNodeId::new();
        let mut directory_name = String::new();
        directory_name
            .try_reserve_exact(MAX_PROFILE_NAME_LEN)
            .map_err(|_| Error::new(Errno::ENOMEM))?;
        // Follow Linux's name mangling, but truncate to leave room for the unique ID.
        // Reference: <https://github.com/torvalds/linux/blob/db2ddb87143519e20a95aa36c60b36107b736a58/security/apparmor/apparmorfs.c#L111>.
        for byte in name.trim_start_matches(['/', '.']).bytes() {
            let byte = match byte {
                b'/' => b'.',
                b' ' => b'_',
                byte if byte.is_ascii_alphanumeric() || b"._-".contains(&byte) => byte,
                _ => continue,
            };
            if directory_name.len() == MAX_PROFILE_NAME_LEN - MAX_DIRECTORY_SUFFIX_LEN {
                break;
            }
            directory_name.push(char::from(byte));
        }
        write!(directory_name, ".{}", id.as_u64()).unwrap();
        Ok(Self {
            name,
            mode,
            raw_data,
            id,
            directory_name: directory_name.into(),
            runtime,
        })
    }

    fn attachment_match(&self, path: &str) -> Result<Option<(usize, usize)>> {
        let Some(runtime) = &self.runtime else {
            return Ok(None);
        };
        let Some(attachment) = &runtime.attachment else {
            return Ok((self.name.starts_with('/') && self.name == path)
                .then_some((usize::MAX, usize::MAX)));
        };
        let Some(prefix_len) =
            dfa::match_attachment(&self.raw_data[attachment.range.clone()], path.as_bytes())?
        else {
            return Ok(None);
        };
        Ok(Some((prefix_len, attachment.minimum_len)))
    }
}

#[derive(Debug)]
pub(super) struct PolicySet {
    profiles: Vec<Arc<Profile>>,
    charged_bytes: usize,
}

impl PolicySet {
    pub(super) fn profiles(&self) -> &[Arc<Profile>] {
        &self.profiles
    }

    fn try_insert(current: Option<&Self>, profile: Profile, charge: usize) -> Result<Self> {
        let current_profiles = current.map_or(&[][..], |set| set.profiles.as_slice());
        if current_profiles
            .iter()
            .any(|current| current.name() == profile.name())
        {
            return Err(Error::new(Errno::EEXIST));
        }

        let current_len = current.map_or(0, |set| set.profiles.len());
        let total_len = current_len
            .checked_add(1)
            .filter(|total| *total <= MAX_LOADED_PROFILES)
            .ok_or_else(capacity_error)?;

        let current_charge = current.map_or(0, |set| set.charged_bytes);
        let charged_bytes = current_charge
            .checked_add(charge)
            .filter(|total| *total <= MAX_POLICY_STORE_BYTES)
            .ok_or_else(capacity_error)?;

        let mut profiles = Vec::new();
        profiles
            .try_reserve_exact(total_len)
            .map_err(|_| Error::new(Errno::ENOMEM))?;
        // Linux visits the most recently loaded profile first when selecting attachments.
        profiles.push(try_new_arc(profile)?);
        profiles.extend_from_slice(current_profiles);

        Ok(Self {
            profiles,
            charged_bytes,
        })
    }
}

pub(super) struct PolicyStore {
    current: Mutex<Option<Arc<PolicySet>>>,
}

impl PolicyStore {
    pub(super) fn load(&self, blob: Vec<u8>) -> Result<()> {
        if blob.is_empty() {
            return Err(Error::new(Errno::EINVAL));
        }
        if blob.len() > MAX_POLICY_BLOB_SIZE {
            return Err(Error::new(Errno::E2BIG));
        }

        let charge = blob.len();
        let profile = abi::decode(blob)?;
        let mut guard = self.current.lock();
        // Publish only after allocation succeeds, leaving the old set intact on error.
        let next = PolicySet::try_insert(guard.as_deref(), profile, charge)?;
        *guard = Some(try_new_arc(next)?);
        Ok(())
    }

    pub(super) fn remove(&self, name: &[u8]) -> Result<()> {
        let name = name.strip_suffix(&[0]).unwrap_or(name);
        let name = abi::validate_profile_name(name)?;
        let mut guard = self.current.lock();
        let current = guard.as_ref().ok_or_else(|| Error::new(Errno::ENOENT))?;
        let index = current
            .profiles
            .iter()
            .position(|profile| profile.name() == name)
            .ok_or_else(|| Error::new(Errno::ENOENT))?;
        if current.profiles.len() == 1 {
            *guard = None;
            return Ok(());
        }

        let mut profiles = Vec::new();
        profiles
            .try_reserve_exact(current.profiles.len() - 1)
            .map_err(|_| Error::new(Errno::ENOMEM))?;
        profiles.extend_from_slice(&current.profiles[..index]);
        profiles.extend_from_slice(&current.profiles[index + 1..]);
        let next = PolicySet {
            profiles,
            charged_bytes: current.charged_bytes - current.profiles[index].raw_data.len(),
        };
        *guard = Some(try_new_arc(next)?);
        Ok(())
    }

    pub(super) fn snapshot(&self) -> Option<Arc<PolicySet>> {
        self.current.lock().clone()
    }

    const fn new() -> Self {
        Self {
            current: Mutex::new(None),
        }
    }

    pub(super) fn contains(&self, profile: &Profile) -> bool {
        let Some(snapshot) = self.snapshot() else {
            return false;
        };
        snapshot
            .profiles
            .iter()
            .any(|current| current.name() == profile.name() && current.id() == profile.id())
    }

    pub(super) fn find_attachment(&self, path: &str) -> Result<Option<Arc<Profile>>> {
        let Some(snapshot) = self.snapshot() else {
            return Ok(None);
        };
        let mut selected = None;
        let mut priority = None;
        let mut ambiguous = false;
        for profile in snapshot.profiles() {
            let Some((prefix_len, minimum_len)) = profile.attachment_match(path)? else {
                continue;
            };
            if priority.is_none_or(|current| prefix_len > current) {
                priority = Some(prefix_len.max(minimum_len));
                selected = Some(profile.clone());
                ambiguous = false;
            } else if priority == Some(prefix_len) {
                ambiguous = true;
            }
        }
        Ok(if ambiguous { None } else { selected })
    }
}

pub(super) static POLICY_STORE: PolicyStore = PolicyStore::new();

fn capacity_error() -> Error {
    Error::new(Errno::ENOSPC)
}

fn try_new_arc<T>(value: T) -> Result<Arc<T>> {
    Arc::try_new(value).map_err(|_| Error::new(Errno::ENOMEM))
}
