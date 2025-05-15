// SPDX-License-Identifier: MPL-2.0

use core::ops::Deref;

use super::Thread;
use crate::prelude::*;

pub struct CurrentThread(pub(super) Arc<Thread>);

pub struct CurrentThreadRef<'a>(pub(super) &'a Arc<Thread>);

impl !Send for CurrentThread {}
impl !Sync for CurrentThread {}

impl !Send for CurrentThreadRef<'_> {}
impl !Sync for CurrentThreadRef<'_> {}

impl Deref for CurrentThread {
    type Target = Arc<Thread>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Deref for CurrentThreadRef<'_> {
    type Target = Arc<Thread>;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl Into<Arc<Thread>> for CurrentThread {
    fn into(self) -> Arc<Thread> {
        self.0
    }
}
