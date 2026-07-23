// SPDX-License-Identifier: MPL-2.0

#![feature(proc_macro_hygiene)]
#![feature(custom_inner_attributes)]

pub mod test_deepest_module_wins {
    pub mod parent {
        pub mod child {
            pub mod parent {
                pub mod child;
                const _: () = child::deepest_wins();
            }
        }
    }
}

pub mod test_override {
    pub mod parent {
        pub mod child {
            pub mod parent {
                pub mod child;
            }
        }
        const _: () = child::parent::child::override_parent();
    }
}

pub mod test_mod_rs_flavor {
    /// The first tow module files are child.rs,
    /// but this child differs by using `child/mod.rs` file style.
    pub mod child;
    type TestAccessibility = child::RecognizeModRs;
}

pub mod test_multiple_arguments {
    pub mod one_ident_and_one_override;
    pub mod two_idents;

    type TestAccessibility1 = one_ident_and_one_override::VisibleToParentToo;
    type TestAccessibility2 = one_ident_and_one_override::VisibleToParent;
    type TestAccessibility3 = two_idents::VisibleToParent;
}

pub mod test_outer_attribute {
    // The outer attribute compiles, works, and doesn't require any nightly feature to work.
    #[short_vis_path::add(parent = crate::test_outer_attribute)]
    pub mod test {
        pub(in parent) type VisibleToParent = ();
    }

    type TestAccessibility = test::VisibleToParent;
}
