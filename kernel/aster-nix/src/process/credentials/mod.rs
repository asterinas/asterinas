// SPDX-License-Identifier: MPL-2.0

pub mod c_types;
pub mod capabilities;
mod credentials_;
mod group;
mod static_cap;
mod user;

use aster_rights::FullOp;
use credentials_::Credentials_;
pub use group::Gid;
pub use user::Uid;

use crate::prelude::*;

/// `Credentials` represents a set of associated numeric user ids (UIDs) and group identifiers (GIDs)
/// for a process.
/// These identifiers are as follows:
/// - real user ID and group ID;
/// - effective user ID and group ID;
/// - saved-set user ID and saved-set group ID;
/// - file system user ID and group ID (Linux-specific);
/// - supplementary group IDs;
/// - Linux capabilities.
pub struct Credentials<R = FullOp>(Arc<Credentials_>, R);
