// SPDX-License-Identifier: MPL-2.0

//! Decoding and storage of the minimal AppArmor ABI v5 policy subset.

use alloc::{string::String, sync::Arc, vec::Vec};

use aster_core::prelude::*;
use ostd::sync::Mutex;

mod abi;

pub(super) const MAX_POLICY_BLOB_SIZE: usize = 1024 * 1024;
const MAX_LOADED_PROFILES: usize = 1024;
const MAX_POLICY_STORE_BYTES: usize = 16 * 1024 * 1024;
const MAX_PROFILE_NAME_LEN: usize = 255;

fn try_new_arc<T>(value: T) -> Result<Arc<T>> {
    Arc::try_new(value).map_err(|_| Error::new(Errno::ENOMEM))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ProfileMode {
    Enforce,
    Complain,
}

impl ProfileMode {
    fn decode(raw: u32) -> Result<Self> {
        match raw {
            0 => Ok(Self::Enforce),
            1 => Ok(Self::Complain),
            2..=4 => Err(Error::new(Errno::EOPNOTSUPP)),
            _ => Err(Error::new(Errno::EPROTO)),
        }
    }

    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Enforce => "enforce",
            Self::Complain => "complain",
        }
    }
}

#[derive(Debug)]
pub(super) struct Profile {
    name: String,
    mode: ProfileMode,
}

impl Profile {
    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn mode(&self) -> ProfileMode {
        self.mode
    }
}

#[derive(Debug)]
pub(super) struct PolicySet {
    profiles: Vec<Arc<Profile>>,
    charged_bytes: usize,
}

impl PolicySet {
    fn try_insert(current: Option<&Self>, profile: Profile, charge: usize) -> Result<Self> {
        Self::try_insert_with(current, profile, charge, try_new_arc)
    }

    fn try_insert_with<F>(
        current: Option<&Self>,
        profile: Profile,
        charge: usize,
        allocate_profile: F,
    ) -> Result<Self>
    where
        F: FnOnce(Profile) -> Result<Arc<Profile>>,
    {
        let current_profiles = current.map_or(&[][..], |set| set.profiles.as_slice());
        let index =
            match current_profiles.binary_search_by(|current| current.name().cmp(profile.name())) {
                Ok(_) => return Err(Error::new(Errno::EEXIST)),
                Err(index) => index,
            };

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
        profiles.extend_from_slice(current_profiles);
        profiles.insert(index, allocate_profile(profile)?);

        Ok(Self {
            profiles,
            charged_bytes,
        })
    }

    pub(super) fn profiles(&self) -> &[Arc<Profile>] {
        &self.profiles
    }
}

pub(super) struct PolicyStore {
    current: Mutex<Option<Arc<PolicySet>>>,
}

impl PolicyStore {
    const fn new() -> Self {
        Self {
            current: Mutex::new(None),
        }
    }

    pub(super) fn load(&self, blob: &[u8]) -> Result<()> {
        self.load_with(blob, try_new_arc, try_new_arc)
    }

    fn load_with<P, S>(&self, blob: &[u8], allocate_profile: P, allocate_set: S) -> Result<()>
    where
        P: FnOnce(Profile) -> Result<Arc<Profile>>,
        S: FnOnce(PolicySet) -> Result<Arc<PolicySet>>,
    {
        if blob.is_empty() {
            return Err(Error::new(Errno::EINVAL));
        }
        if blob.len() > MAX_POLICY_BLOB_SIZE {
            return Err(Error::new(Errno::E2BIG));
        }

        let profile = abi::decode(blob)?;
        let mut guard = self.current.lock();
        let next =
            PolicySet::try_insert_with(guard.as_deref(), profile, blob.len(), allocate_profile)?;
        *guard = Some(allocate_set(next)?);
        Ok(())
    }

    pub(super) fn snapshot(&self) -> Option<Arc<PolicySet>> {
        self.current.lock().clone()
    }
}

pub(super) static POLICY_STORE: PolicyStore = PolicyStore::new();

fn capacity_error() -> Error {
    Error::new(Errno::ENOSPC)
}

#[cfg(ktest)]
mod tests {
    use alloc::format;
    use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use ostd::{
        prelude::ktest,
        task::{Task, TaskOptions},
    };

    use super::*;

    fn push_u32(output: &mut Vec<u8>, value: u32) {
        output.push(2);
        output.extend_from_slice(&value.to_le_bytes());
    }

    fn push_chunk(output: &mut Vec<u8>, code: u8, value: &[u8]) {
        output.push(code);
        output.extend_from_slice(&((value.len() + 1) as u16).to_le_bytes());
        output.extend_from_slice(value);
        output.push(0);
    }

    fn push_name(output: &mut Vec<u8>, name: &str) {
        push_chunk(output, 4, name.as_bytes());
    }

    fn push_named_struct(output: &mut Vec<u8>, name: &str) {
        push_name(output, name);
        output.push(7);
    }

    fn policy_unit(name: &str) -> Vec<u8> {
        let mut output = Vec::new();
        push_name(&mut output, "version");
        push_u32(&mut output, 5);
        push_named_struct(&mut output, "profile");
        push_chunk(&mut output, 5, name.as_bytes());
        push_named_struct(&mut output, "flags");
        push_u32(&mut output, 0);
        push_u32(&mut output, 0);
        push_u32(&mut output, 0);
        output.push(8);
        for _ in 0..4 {
            push_u32(&mut output, 0);
        }
        push_named_struct(&mut output, "caps64");
        for _ in 0..4 {
            push_u32(&mut output, 0);
        }
        output.push(8);
        output.push(8);
        output
    }

    fn profile(name: &str) -> Profile {
        Profile {
            name: name.into(),
            mode: ProfileMode::Enforce,
        }
    }

    fn assert_errno<T>(result: Result<T>, expected: Errno) {
        assert_eq!(result.err().unwrap().error(), expected);
    }

    fn assert_names(set: &PolicySet, expected: &[&str]) {
        let actual: Vec<&str> = set
            .profiles()
            .iter()
            .map(|profile| profile.name())
            .collect();
        assert_eq!(actual, expected);
    }

    fn fail_allocation<T>(_value: T) -> Result<Arc<T>> {
        Err(Error::new(Errno::ENOMEM))
    }

    #[ktest]
    fn sequential_loads_produce_sorted_snapshots() {
        let store = PolicyStore::new();
        assert!(store.snapshot().is_none());
        store.load(&policy_unit("second")).unwrap();
        store.load(&policy_unit("first")).unwrap();
        assert_names(&store.snapshot().unwrap(), &["first", "second"]);
    }

    #[ktest]
    fn duplicate_or_decoder_failure_preserves_snapshot() {
        let store = PolicyStore::new();
        store.load(&policy_unit("same")).unwrap();
        let before = store.snapshot().unwrap();

        assert_errno(store.load(&policy_unit("same")), Errno::EEXIST);
        let after_conflict = store.snapshot().unwrap();
        assert!(Arc::ptr_eq(&before, &after_conflict));

        assert_errno(store.load(&[0]), Errno::EPROTO);
        let after_decode = store.snapshot().unwrap();
        assert!(Arc::ptr_eq(&before, &after_decode));
        assert_names(&after_decode, &["same"]);
    }

    #[ktest]
    fn allocation_failure_preserves_snapshot() {
        let store = PolicyStore::new();
        store.load(&policy_unit("old")).unwrap();
        let before = store.snapshot().unwrap();

        assert_errno(
            store.load_with(&policy_unit("profile-oom"), fail_allocation, try_new_arc),
            Errno::ENOMEM,
        );
        let after_profile_oom = store.snapshot().unwrap();
        assert!(Arc::ptr_eq(&before, &after_profile_oom));

        assert_errno(
            store.load_with(&policy_unit("set-oom"), try_new_arc, fail_allocation),
            Errno::ENOMEM,
        );
        let after_set_oom = store.snapshot().unwrap();
        assert!(Arc::ptr_eq(&before, &after_set_oom));
        assert_names(&after_set_oom, &["old"]);
    }

    #[ktest]
    fn enforces_input_profile_and_charge_limits() {
        let store = PolicyStore::new();
        assert_errno(store.load(&[]), Errno::EINVAL);
        assert_errno(
            store.load(&alloc::vec![0; MAX_POLICY_BLOB_SIZE + 1]),
            Errno::E2BIG,
        );

        let mut profiles = Vec::new();
        for index in 0..MAX_LOADED_PROFILES - 1 {
            profiles.push(Arc::new(profile(&format!("profile-{index:04}"))));
        }
        let almost_full = PolicySet {
            profiles,
            charged_bytes: 1,
        };
        let full = PolicySet::try_insert(Some(&almost_full), profile("profile-last"), 1).unwrap();
        assert_eq!(full.profiles().len(), MAX_LOADED_PROFILES);
        assert_errno(
            PolicySet::try_insert(Some(&full), profile("too-many"), 1),
            Errno::ENOSPC,
        );

        assert_errno(
            PolicySet::try_insert(None, profile("large"), MAX_POLICY_STORE_BYTES + 1),
            Errno::ENOSPC,
        );
    }

    #[ktest]
    fn duplicate_name_wins_over_profile_limit() {
        let mut profiles = Vec::new();
        for index in 0..MAX_LOADED_PROFILES {
            profiles.push(Arc::new(profile(&format!("profile-{index:04}"))));
        }
        let full = PolicySet {
            profiles,
            charged_bytes: 1,
        };

        assert_errno(
            PolicySet::try_insert(Some(&full), profile("profile-0000"), 1),
            Errno::EEXIST,
        );
    }

    #[ktest]
    fn duplicate_name_wins_over_charge_limit() {
        let full = PolicySet {
            profiles: alloc::vec![Arc::new(profile("existing"))],
            charged_bytes: MAX_POLICY_STORE_BYTES,
        };

        assert_errno(
            PolicySet::try_insert(Some(&full), profile("existing"), 1),
            Errno::EEXIST,
        );
    }

    #[ktest]
    fn concurrent_writers_do_not_lose_profiles() {
        let store = Arc::new(PolicyStore::new());
        let done = Arc::new(AtomicUsize::new(0));

        for name in ["first", "second"] {
            let store = store.clone();
            let done = done.clone();
            TaskOptions::new(move || {
                store.load(&policy_unit(name)).unwrap();
                done.fetch_add(1, Ordering::Release);
            })
            .spawn()
            .unwrap();
        }

        while done.load(Ordering::Acquire) != 2 {
            Task::yield_now();
        }
        assert_names(&store.snapshot().unwrap(), &["first", "second"]);
    }

    #[ktest]
    fn reader_observes_only_complete_snapshots() {
        let store = Arc::new(PolicyStore::new());
        store.load(&policy_unit("old")).unwrap();
        let done = Arc::new(AtomicBool::new(false));

        let writer_store = store.clone();
        let writer_done = done.clone();
        TaskOptions::new(move || {
            writer_store.load(&policy_unit("new")).unwrap();
            writer_done.store(true, Ordering::Release);
        })
        .spawn()
        .unwrap();

        loop {
            let snapshot = store.snapshot().unwrap();
            match snapshot.profiles().len() {
                1 => assert_names(&snapshot, &["old"]),
                2 => assert_names(&snapshot, &["new", "old"]),
                _ => panic!("partial snapshot"),
            }
            if done.load(Ordering::Acquire) {
                break;
            }
            Task::yield_now();
        }
        assert_names(&store.snapshot().unwrap(), &["new", "old"]);
    }
}
