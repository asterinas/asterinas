// SPDX-License-Identifier: MPL-2.0

pub mod c_types;
pub mod capabilities;
mod credentials_;
mod group;
mod secure_bits;
mod static_cap;
mod user;

use aster_rights::FullOp;
use credentials_::Credentials_;
pub use group::Gid;
pub use secure_bits::SecureBits;
pub use user::Uid;

use crate::prelude::*;

/// A set of associated numeric user IDs (UIDs) and group IDs (GIDs) for a process.
///
/// This type contains:
/// - real user ID and group ID;
/// - effective user ID and group ID;
/// - saved-set user ID and saved-set group ID;
/// - filesystem user ID and group ID (Linux-specific);
/// - supplementary group IDs;
/// - Linux capabilities;
/// - secure bits.
pub struct Credentials<R = FullOp>(Arc<Credentials_>, R);
