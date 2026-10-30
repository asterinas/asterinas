// SPDX-License-Identifier: MPL-2.0

//! An immutable profile reference for a singleton label.
//! `None` represents unconfined.

use alloc::string::String;

use aster_core::{prelude::*, security::lsm::LsmState};

use crate::policy::Profile;

pub(super) fn current_attr(state: Option<&LsmState>) -> Result<String> {
    let mut output = String::new();
    if let Some(state) = state {
        let profile = state
            .downcast_ref::<Profile>()
            .ok_or_else(|| Error::new(Errno::EINVAL))?;
        let mode = profile.mode().as_str();
        output
            .try_reserve_exact(profile.name().len() + mode.len() + 4)
            .map_err(|_| Error::new(Errno::ENOMEM))?;
        output.push_str(profile.name());
        output.push_str(" (");
        output.push_str(mode);
        output.push_str(")\n");
    } else {
        output
            .try_reserve_exact(11)
            .map_err(|_| Error::new(Errno::ENOMEM))?;
        output.push_str("unconfined\n");
    }
    Ok(output)
}

#[cfg(ktest)]
mod tests {
    use alloc::sync::Arc;

    use ostd::prelude::ktest;

    use super::*;
    use crate::policy::PolicyStore;

    #[ktest]
    fn formats_unconfined_without_installing_state() {
        assert_eq!(current_attr(None).unwrap(), "unconfined\n");
    }

    #[ktest]
    fn formats_the_owned_profile_and_mode() {
        let store = PolicyStore::new();
        store.load(include_bytes!(
            "../../../../test/initramfs/src/regression/security/lsm/apparmor-fixtures/observe.bin"
        )).unwrap();
        let snapshot = store.snapshot().unwrap();
        let state: LsmState = snapshot.profiles()[0].clone();
        assert_eq!(current_attr(Some(&state)).unwrap(), "observe (complain)\n");
        assert_eq!(
            current_attr(Some(&(Arc::new(7usize) as _)))
                .unwrap_err()
                .error(),
            Errno::EINVAL
        );
    }
}
