// SPDX-License-Identifier: MPL-2.0

/// Defines a struct representing a boolean value.
///
/// In some cases, it is beneficial to use a struct instead of
/// a plain boolean value to clarify the semantics.
/// This macro provides a convenient way to define a struct
/// that represents a boolean value.
#[macro_export]
macro_rules! define_boolean_value {
    (
        $(#[$attr:meta])*
        $name: ident
    ) => {
        $(#[$attr])*
        #[derive(Debug, Clone, Copy)]
        pub struct $name(bool);

        impl $name {
            pub const TRUE: Self = Self(true);
            pub const FALSE: Self = Self(false);
        }

        impl core::ops::Deref for $name {
            type Target = bool;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
    };
}
