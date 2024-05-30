// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::AtomicBool;

use atomic::Ordering;

use super::addr::FamilyId;
use crate::{
    events::IoEvents,
    process::signal::{CanPoll, Pollee},
};

/// An unbound netlink socket
pub struct UnboundNetlink {
    is_nonblocking: AtomicBool,
    family_id: FamilyId,
    pollee: Pollee,
}

impl UnboundNetlink {
    pub fn new(is_nonblocking: bool, family_id: FamilyId) -> Self {
        Self {
            is_nonblocking: AtomicBool::new(is_nonblocking),
            family_id,
            pollee: Pollee::new(IoEvents::empty()),
        }
    }

    pub fn family_id(&self) -> FamilyId {
        self.family_id
    }

    pub fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Relaxed)
    }
}

impl CanPoll for UnboundNetlink {
    fn poll_object(&self) -> &dyn CanPoll {
        &self.pollee
    }
}
