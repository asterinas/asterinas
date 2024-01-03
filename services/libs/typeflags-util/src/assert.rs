// SPDX-License-Identifier: MPL-2.0

//! define macro assert_type_same

use crate::same::SameAs;

pub type AssertTypeSame<Lhs, Rhs> = <Lhs as SameAs<Rhs>>::Output;

#[macro_export]
macro_rules! assert_type_same {
    ($lhs:ty, $rhs:ty) => {
        const _: $crate::assert::AssertTypeSame<$lhs, $rhs> = $crate::True;
    };
}

#[cfg(test)]
mod test {
    #[test]
    fn test() {
        assert_type_same!(u16, u16);
        assert_type_same!((), ());
    }
}
