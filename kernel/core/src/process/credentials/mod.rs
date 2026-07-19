// SPDX-License-Identifier: MPL-2.0

pub(crate) mod c_types;
pub(crate) mod capabilities;
mod credentials_;
mod file_capabilities;
mod group;
mod secure_bits;
mod static_cap;
mod user;

use aster_rights::FullOp;
use credentials_::Credentials_;
pub(super) use credentials_::ExecCred;
pub(crate) use file_capabilities::{FileCapabilities, VfsCapRevision};
pub(crate) use group::Gid;
pub(crate) use secure_bits::SecureBits;
pub(crate) use user::Uid;

use crate::prelude::*;

/// A set of associated numeric user IDs (UIDs) and group IDs (GIDs) for a process.
///
/// This type contains:
/// - real user ID and group ID;
/// - effective user ID and group ID;
/// - saved-set user ID and group ID;
/// - filesystem user ID and group ID (Linux-specific);
/// - supplementary group IDs;
/// - Linux capabilities;
/// - secure bits.
pub(crate) struct Credentials<R = FullOp>(Arc<Credentials_>, R);
