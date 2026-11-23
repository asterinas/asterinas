// SPDX-License-Identifier: MPL-2.0

//! A snapshot of the credentials that protect existing perf events.

use aster_rights::ReadOp;

use crate::process::{
    Credentials,
    credentials::{Gid, Uid, capabilities::CapSet},
};

pub(super) struct AccessCredentials {
    uids: [Uid; 3],
    gids: [Gid; 3],
    permitted: CapSet,
    effective: CapSet,
}

impl AccessCredentials {
    pub(super) fn read(credentials: &Credentials<ReadOp>) -> Self {
        Self {
            uids: [credentials.ruid(), credentials.euid(), credentials.suid()],
            gids: [credentials.rgid(), credentials.egid(), credentials.sgid()],
            permitted: credentials.permitted_capset(),
            effective: credentials.effective_capset(),
        }
    }

    pub(super) fn invalidated_by(&self, updated: &Self) -> bool {
        self.uids != updated.uids
            || self.gids != updated.gids
            || !self.permitted.contains(updated.permitted)
            || !self.effective.contains(updated.effective)
    }
}
