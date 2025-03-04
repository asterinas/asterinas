// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use crate::{
    mm::{CachePolicy, PageFlags, PageProperty, PrivilegedPageFlags},
    prelude::*,
};

mod page_prop {
    use super::*;

    /// Ensures `PageProperty::new` correctly initializes a `PageProperty`.
    #[ktest]
    fn page_property_new() {
        let flags = PageFlags::R | PageFlags::W;
        let cache = CachePolicy::Writeback;
        let page_property = PageProperty::new(flags, cache);

        assert_eq!(page_property.flags, flags);
        assert_eq!(page_property.cache, cache);
        assert_eq!(page_property.priv_flags, PrivilegedPageFlags::USER);
    }

    /// Ensures `PageProperty::new_absent` initializes an invalid `PageProperty`.
    #[ktest]
    fn page_property_new_absent() {
        let page_property = PageProperty::new_absent();

        assert_eq!(page_property.flags, PageFlags::empty());
        assert_eq!(page_property.cache, CachePolicy::Writeback);
        assert_eq!(page_property.priv_flags, PrivilegedPageFlags::empty());
    }

    /// Verifies each variant of the `CachePolicy` enum.
    #[ktest]
    fn cache_policy_enum() {
        assert_eq!(CachePolicy::Uncacheable as u8, 0);
        assert_eq!(CachePolicy::WriteCombining as u8, 1);
        assert_eq!(CachePolicy::WriteProtected as u8, 2);
        assert_eq!(CachePolicy::Writethrough as u8, 3);
        assert_eq!(CachePolicy::Writeback as u8, 4);
    }

    /// Verifies the basic functionality of `PageFlags` bitflags.
    #[ktest]
    fn page_flags_basic() {
        let flags = PageFlags::R;
        assert!(flags.contains(PageFlags::R));
        assert!(!flags.contains(PageFlags::W));
        assert!(!flags.contains(PageFlags::X));

        let flags = PageFlags::RWX;
        assert!(flags.contains(PageFlags::R));
        assert!(flags.contains(PageFlags::W));
        assert!(flags.contains(PageFlags::X));
    }

    /// Ensures `PageFlags` combinations are correct.
    #[ktest]
    fn page_flags_combinations() {
        let rw = PageFlags::R | PageFlags::W;
        assert_eq!(rw, PageFlags::RW);

        let rx = PageFlags::R | PageFlags::X;
        assert_eq!(rx, PageFlags::RX);

        let rwx = PageFlags::R | PageFlags::W | PageFlags::X;
        assert_eq!(rwx, PageFlags::RWX);
    }

    /// Verifies the accessed and dirty bits of `PageFlags`.
    #[ktest]
    fn page_flags_accessed_dirty() {
        let flags = PageFlags::ACCESSED;
        assert!(flags.contains(PageFlags::ACCESSED));
        assert!(!flags.contains(PageFlags::DIRTY));

        let flags = PageFlags::DIRTY;
        assert!(flags.contains(PageFlags::DIRTY));
        assert!(!flags.contains(PageFlags::ACCESSED));

        let flags = PageFlags::ACCESSED | PageFlags::DIRTY;
        assert!(flags.contains(PageFlags::ACCESSED));
        assert!(flags.contains(PageFlags::DIRTY));
    }

    /// Verifies the available bits of `PageFlags`.
    #[ktest]
    fn page_flags_available() {
        let flags = PageFlags::AVAIL1;
        assert!(flags.contains(PageFlags::AVAIL1));
        assert!(!flags.contains(PageFlags::AVAIL2));

        let flags = PageFlags::AVAIL2;
        assert!(flags.contains(PageFlags::AVAIL2));
        assert!(!flags.contains(PageFlags::AVAIL1));

        let flags = PageFlags::AVAIL1 | PageFlags::AVAIL2;
        assert!(flags.contains(PageFlags::AVAIL1));
        assert!(flags.contains(PageFlags::AVAIL2));
    }

    /// Verifies the basic functionality of `PrivilegedPageFlags`.
    #[ktest]
    fn privileged_page_flags_basic() {
        let flags = PrivilegedPageFlags::USER;
        assert!(flags.contains(PrivilegedPageFlags::USER));
        assert!(!flags.contains(PrivilegedPageFlags::GLOBAL));

        let flags = PrivilegedPageFlags::GLOBAL;
        assert!(flags.contains(PrivilegedPageFlags::GLOBAL));
        assert!(!flags.contains(PrivilegedPageFlags::USER));
    }

    /// Ensures `PrivilegedPageFlags` combinations are correct.
    #[ktest]
    fn privileged_page_flags_combinations() {
        let flags = PrivilegedPageFlags::USER | PrivilegedPageFlags::GLOBAL;
        // Since `bitflags` implements `Debug` and `PartialEq` for `PrivilegedPageFlags`, we can directly compare
        let expected = PrivilegedPageFlags::USER | PrivilegedPageFlags::GLOBAL;
        assert_eq!(flags, expected);
    }

    /// Verifies the `PrivilegedPageFlags::SHARED` flag under specific configurations.
    #[ktest]
    #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
    fn privileged_page_flags_shared_enabled() {
        let flags = PrivilegedPageFlags::SHARED;
        assert!(flags.contains(PrivilegedPageFlags::SHARED));
    }

    /// Ensures `PrivilegedPageFlags::SHARED` is unavailable when conditions are not met.
    #[ktest]
    #[cfg(not(all(target_arch = "x86_64", feature = "cvm_guest")))]
    fn privileged_page_flags_shared_disabled() {
        // Since the `SHARED` flag is undefined when conditions are not met,
        // we cannot directly test its absence, but we can ensure the code compiles.
        let flags = PrivilegedPageFlags::USER | PrivilegedPageFlags::GLOBAL;
        assert!(flags.contains(PrivilegedPageFlags::USER));
        assert!(flags.contains(PrivilegedPageFlags::GLOBAL));
    }

    /// Verifies the `PageProperty` Debug output.
    #[ktest]
    fn page_property_debug() {
        let flags = PageFlags::RW | PageFlags::DIRTY;
        let cache = CachePolicy::WriteProtected;
        let page_property = PageProperty::new(flags, cache);

        let debug_str = format!("{:?}", page_property);
        assert!(debug_str.contains("flags"));
        assert!(debug_str.contains("RW"));
        assert!(debug_str.contains("DIRTY"));
        assert!(debug_str.contains("WriteProtected"));
    }

    /// Ensures `PageProperty` implements `PartialEq` and `Eq` correctly.
    #[ktest]
    fn page_property_equality() {
        let flags1 = PageFlags::R | PageFlags::W;
        let cache1 = CachePolicy::Writeback;
        let page_property1 = PageProperty::new(flags1, cache1);

        let flags2 = PageFlags::R | PageFlags::W;
        let cache2 = CachePolicy::Writeback;
        let page_property2 = PageProperty::new(flags2, cache2);

        assert_eq!(page_property1, page_property2);

        let page_property3 = PageProperty::new_absent();
        assert_ne!(page_property1, page_property3);
    }

    /// Verifies bit operations for `PageFlags`.
    #[ktest]
    fn page_flags_bit_operations() {
        let mut flags = PageFlags::empty();
        flags.insert(PageFlags::R);
        assert!(flags.contains(PageFlags::R));
        assert!(!flags.contains(PageFlags::W));

        flags.insert(PageFlags::W);
        assert!(flags.contains(PageFlags::R));
        assert!(flags.contains(PageFlags::W));

        flags.remove(PageFlags::R);
        assert!(!flags.contains(PageFlags::R));
        assert!(flags.contains(PageFlags::W));
    }

    /// Verifies bit operations for `PrivilegedPageFlags`.
    #[ktest]
    fn privileged_page_flags_bit_operations() {
        let mut flags = PrivilegedPageFlags::empty();
        flags.insert(PrivilegedPageFlags::USER);
        assert!(flags.contains(PrivilegedPageFlags::USER));
        assert!(!flags.contains(PrivilegedPageFlags::GLOBAL));

        flags.insert(PrivilegedPageFlags::GLOBAL);
        assert!(flags.contains(PrivilegedPageFlags::USER));
        assert!(flags.contains(PrivilegedPageFlags::GLOBAL));

        flags.remove(PrivilegedPageFlags::USER);
        assert!(!flags.contains(PrivilegedPageFlags::USER));
        assert!(flags.contains(PrivilegedPageFlags::GLOBAL));
    }
}
