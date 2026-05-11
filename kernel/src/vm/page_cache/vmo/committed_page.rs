// SPDX-License-Identifier: MPL-2.0

use core::ops::Deref;

use crate::vm::page_cache::{CachePage, CachePageExt, cache_page::LockedCachePage};

/// A committed VMO page that may temporarily keep the page lock.
#[derive(Debug)]
pub(super) enum CommittedPage {
    /// The committed page does not need to carry the page lock.
    Unlocked(CachePage),
    /// The committed page keeps the page lock for a following state transition.
    Locked(LockedCachePage),
}

impl CommittedPage {
    /// Returns the underlying cache page.
    pub(super) fn page(&self) -> &CachePage {
        match self {
            Self::Unlocked(page) => page,
            Self::Locked(locked_page) => locked_page,
        }
    }

    /// Converts this committed page into an unlocked cache page.
    pub(super) fn into_page(self) -> CachePage {
        match self {
            Self::Unlocked(page) => page,
            Self::Locked(locked_page) => locked_page.unlock(),
        }
    }

    /// Converts this committed page into a locked cache page.
    pub(super) fn into_locked(self) -> LockedCachePage {
        match self {
            Self::Unlocked(page) => page.lock(),
            Self::Locked(locked_page) => locked_page,
        }
    }
}

impl From<CachePage> for CommittedPage {
    fn from(page: CachePage) -> Self {
        Self::Unlocked(page)
    }
}

impl From<LockedCachePage> for CommittedPage {
    fn from(locked_page: LockedCachePage) -> Self {
        Self::Locked(locked_page)
    }
}

impl Deref for CommittedPage {
    type Target = CachePage;

    fn deref(&self) -> &Self::Target {
        self.page()
    }
}
