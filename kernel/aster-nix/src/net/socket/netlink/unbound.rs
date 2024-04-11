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
    family: FamilyId,
    pollee: Pollee,
}

impl UnboundNetlink {
    pub fn new(is_nonblocking: bool, family: FamilyId) -> Self {
        Self {
            is_nonblocking: AtomicBool::new(is_nonblocking),
            family,
            pollee: Pollee::new(IoEvents::empty()),
        }
    }

    pub fn family(&self) -> FamilyId {
        self.family
    }

    pub fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Relaxed)
    }
}

impl CanPoll for UnboundNetlink {
    fn pollee(&self) -> &Pollee {
        &self.pollee
    }
}
