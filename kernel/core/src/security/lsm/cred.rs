// SPDX-License-Identifier: MPL-2.0

//! Immutable, module-owned credential security state.

use super::modules;
use crate::prelude::*;

/// An immutable credential state owned and interpreted by one LSM.
pub type LsmState = Arc<dyn Any + Send + Sync>;

/// An empty snapshot has no heap allocation, including no slot container.
/// Clones share immutable entries; each credential owns its own replaceable slot.
#[derive(Clone, Debug, Default)]
pub(crate) struct CredSecurity(Option<Arc<Vec<(&'static str, LsmState)>>>);

impl CredSecurity {
    pub(crate) fn init() -> Result<Self> {
        Self::collect(
            modules::active_modules()
                .iter()
                .map(|module| (module.name(), module.init_cred_state())),
        )
    }

    pub(crate) fn get(&self, name: &str) -> Option<&LsmState> {
        self.0
            .as_ref()?
            .iter()
            .find(|(module, _)| *module == name)
            .map(|(_, state)| state)
    }

    pub(crate) fn needs_exec_path(&self) -> bool {
        modules::active_modules()
            .iter()
            .any(|module| module.needs_exec_path(self.get(module.name())))
    }

    pub(crate) fn prepare_exec(&self, path: Option<&str>) -> Result<Self> {
        Self::collect(modules::active_modules().iter().map(|module| {
            (
                module.name(),
                module.prepare_exec_state(self.get(module.name()), path),
            )
        }))
    }

    fn collect(
        states: impl IntoIterator<Item = (&'static str, Result<Option<LsmState>>)>,
    ) -> Result<Self> {
        let mut entries = Vec::new();
        for (name, state) in states {
            if let Some(state) = state? {
                entries
                    .try_reserve(1)
                    .map_err(|_| Error::new(Errno::ENOMEM))?;
                entries.push((name, state));
            }
        }
        if entries.is_empty() {
            return Ok(Self::default());
        }
        Ok(Self(Some(
            Arc::try_new(entries).map_err(|_| Error::new(Errno::ENOMEM))?,
        )))
    }
}

#[cfg(ktest)]
mod tests {
    use core::sync::atomic::{AtomicUsize, Ordering};

    use ostd::prelude::ktest;

    use super::*;

    #[ktest]
    fn empty_state_needs_no_container() {
        let state = CredSecurity::collect([("a", Ok(None)), ("b", Ok(None))]).unwrap();
        let child = state.clone();
        assert!(state.0.is_none());
        assert!(child.0.is_none());
        assert!(state.get("a").is_none());
    }

    #[ktest]
    fn copied_snapshots_keep_module_states_separate() {
        let a: LsmState = Arc::new(11usize);
        let b: LsmState = Arc::new(22usize);
        let parent = CredSecurity::collect([("a", Ok(Some(a))), ("b", Ok(Some(b)))]).unwrap();
        let mut child = parent.clone();
        assert_eq!(child.get("a").unwrap().downcast_ref::<usize>(), Some(&11));
        assert_eq!(child.get("b").unwrap().downcast_ref::<usize>(), Some(&22));
        child = CredSecurity::default();
        assert!(child.get("a").is_none());
        assert_eq!(parent.get("a").unwrap().downcast_ref::<usize>(), Some(&11));
    }

    #[ktest]
    fn failed_collection_releases_candidate_and_preserves_parent() {
        struct CountDrop(Arc<AtomicUsize>);
        impl Drop for CountDrop {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let value: LsmState = Arc::new(CountDrop(drops.clone()));
        let parent = CredSecurity::collect([("a", Ok(Some(value)))]).unwrap();
        let candidate = CredSecurity::collect([
            ("a", Ok(parent.get("a").cloned())),
            ("b", Err(Error::new(Errno::ENOMEM))),
        ]);
        assert_eq!(candidate.unwrap_err().error(), Errno::ENOMEM);
        assert_eq!(drops.load(Ordering::Relaxed), 0);
        let child = parent.clone();
        drop(parent);
        assert_eq!(drops.load(Ordering::Relaxed), 0);
        drop(child);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
    }
}
