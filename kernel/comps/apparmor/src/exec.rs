// SPDX-License-Identifier: MPL-2.0

use aster_core::{prelude::*, security::lsm::LsmState};

use crate::policy::PolicySet;

pub(super) fn prepare(
    current: Option<&LsmState>,
    path: Option<&str>,
    policies: Option<&PolicySet>,
) -> Result<Option<LsmState>> {
    if let Some(current) = current {
        return Ok(Some(current.clone()));
    }
    Ok(path
        .and_then(|path| policies?.find_attachment(path))
        .map(|profile| profile as LsmState))
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::*;
    use crate::{label, policy::PolicyStore};

    #[ktest]
    fn attaches_only_on_exact_match_and_preserves_existing_label() {
        let store = PolicyStore::new();
        store
            .load(include_bytes!(
                "../../../../test/initramfs/src/regression/security/lsm/apparmor-fixtures/demo1.bin"
            ))
            .unwrap();
        store
            .load(include_bytes!(
                "../../../../test/initramfs/src/regression/security/lsm/apparmor-fixtures/demo2.bin"
            ))
            .unwrap();
        let snapshot = store.snapshot().unwrap();
        let first = prepare(None, Some("/usr/bin/demo1"), Some(&snapshot)).unwrap();
        assert_eq!(
            label::current_attr(first.as_ref()).unwrap(),
            "demo1 (enforce)\n"
        );
        for path in [
            None,
            Some("/usr/bin/demo"),
            Some("/usr/bin/demo1-extra"),
            Some("/usr/bin/demo1/child"),
        ] {
            assert!(prepare(None, path, Some(&snapshot)).unwrap().is_none());
            assert_eq!(
                label::current_attr(
                    prepare(first.as_ref(), path, Some(&snapshot))
                        .unwrap()
                        .as_ref()
                )
                .unwrap(),
                "demo1 (enforce)\n"
            );
        }
        let kept = prepare(first.as_ref(), Some("/usr/bin/demo2"), Some(&snapshot)).unwrap();
        assert_eq!(
            label::current_attr(kept.as_ref()).unwrap(),
            "demo1 (enforce)\n"
        );
        let second = prepare(None, Some("/usr/bin/demo2"), Some(&snapshot)).unwrap();
        assert_eq!(
            label::current_attr(second.as_ref()).unwrap(),
            "demo2 (enforce)\n"
        );
    }
}
