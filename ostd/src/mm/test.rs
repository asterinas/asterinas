// SPDX-License-Identifier: MPL-2.0

use crate::{
    mm::{CachePolicy, PageFlags, PageProperty, PrivilegedPageFlags},
    prelude::*,
};

mod page_prop {
    use super::*;

    /// Verifies all functionality of `PageProperty`.
    #[ktest]
    fn page_property() {
        let flags = PageFlags::RWX;
        let cache = CachePolicy::Writeback;
        let priv_flags = PrivilegedPageFlags::USER;

        let prop = PageProperty {
            flags,
            cache,
            priv_flags,
        };

        assert_eq!(prop.flags, flags);
        assert_eq!(prop.cache, cache);
        assert_eq!(prop.priv_flags, priv_flags);

        let new_prop = PageProperty::new(flags, cache);
        assert_eq!(new_prop.flags, flags);
        assert_eq!(new_prop.cache, cache);
        assert_eq!(new_prop.priv_flags, PrivilegedPageFlags::USER);

        let absent_prop = PageProperty::new_absent();
        assert_eq!(absent_prop.flags, PageFlags::empty());
        assert_eq!(absent_prop.cache, CachePolicy::Writeback);
        assert_eq!(absent_prop.priv_flags, PrivilegedPageFlags::empty());
    }

    /// Verifies our custom PageFlags behavior.
    #[ktest]
    fn page_flags() {
        // Test our specific flag combinations
        assert!(PageFlags::RWX.contains(PageFlags::R | PageFlags::W | PageFlags::X));
        assert!((PageFlags::ACCESSED | PageFlags::DIRTY)
            .contains(PageFlags::ACCESSED | PageFlags::DIRTY));
        assert!(
            (PageFlags::AVAIL1 | PageFlags::AVAIL2).contains(PageFlags::AVAIL1 | PageFlags::AVAIL2)
        );
    }

    /// Verifies our custom PrivilegedPageFlags behavior.
    #[ktest]
    fn privileged_page_flags() {
        assert!((PrivilegedPageFlags::USER | PrivilegedPageFlags::GLOBAL)
            .contains(PrivilegedPageFlags::USER | PrivilegedPageFlags::GLOBAL));

        #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
        {
            assert!(PrivilegedPageFlags::SHARED.contains(PrivilegedPageFlags::SHARED));
        }
    }
}
