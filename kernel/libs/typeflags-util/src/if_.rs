// SPDX-License-Identifier: MPL-2.0

//! Type Level If

use crate::bool::{False, True};

pub trait If<B1, B2> {
    type Output;
}

impl<B1, B2> If<B1, B2> for True {
    type Output = B1;
}

impl<B1, B2> If<B1, B2> for False {
    type Output = B2;
}

pub type IfOp<Cond, B1, B2> = <Cond as If<B1, B2>>::Output;

#[cfg(test)]
mod test {
    use super::*;
    use crate::*;

    #[test]
    fn test() {
        assert_type_same!(IfOp<True, u32, ()>, u32);
        assert_type_same!(IfOp<False, (), usize>, usize);
    }
}
