// SPDX-License-Identifier: MPL-2.0

mod condvar;

#[expect(unused_imports)]
pub(crate) use self::condvar::{Condvar, LockErr};
