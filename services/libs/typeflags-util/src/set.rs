// SPDX-License-Identifier: MPL-2.0

//! Common types and traits to deal with type-level sets

use core::{marker::PhantomData, ops::BitOr as Or};

use crate::{
    if_::{If, IfOp},
    And, AndOp, False, OrOp, SameAs, SameAsOp, True,
};

/// A marker trait for type-level sets.
pub trait Set {}

/// An non-empty type-level set.
#[derive(Debug, Clone, Copy)]
pub struct Cons<T, S: Set>(PhantomData<(T, S)>);

impl<T, S: Set> Cons<T, S> {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Cons(PhantomData)
    }
}

/// An empty type-level set.
#[derive(Debug, Clone, Copy)]
pub struct Nil;

impl<T, S: Set> Set for Cons<T, S> {}
impl Set for Nil {}

/// A trait operator to check if `T` is a member of a type set;
pub trait SetContain<T> {
    type Output;
}

pub type SetContainOp<Set, Item> = <Set as SetContain<Item>>::Output;

impl<T> SetContain<T> for Nil {
    type Output = False;
}

impl<T, U, S> SetContain<T> for Cons<U, S>
where
    S: Set + SetContain<T>,
    U: SameAs<T>,
    SameAsOp<U, T>: Or<SetContainOp<S, T>>,
{
    type Output = OrOp<SameAsOp<U, T>, SetContainOp<S, T>>;
}

/// A trait operator to check if a set A includes a set B, i.e., A is a superset of B.
pub trait SetInclude<S: Set> {
    type Output;
}

pub type SetIncludeOp<Super, Sub> = <Super as SetInclude<Sub>>::Output;

impl SetInclude<Nil> for Nil {
    type Output = True;
}

impl<T, S: Set> SetInclude<Cons<T, S>> for Nil {
    type Output = False;
}

impl<T, S: Set> SetInclude<Nil> for Cons<T, S> {
    type Output = True;
}

impl<SuperT, SuperS, SubT, SubS> SetInclude<Cons<SubT, SubS>> for Cons<SuperT, SuperS>
where
    SubS: Set,
    SuperS: Set + SetInclude<SubS> + SetContain<SubT>,
    SuperT: SameAs<SubT>,
    SetContainOp<SuperS, SubT>: And<SetIncludeOp<SuperS, SubS>>,
    SameAsOp<SuperT, SubT>: If<
        SetIncludeOp<SuperS, SubS>,
        AndOp<SetContainOp<SuperS, SubT>, SetIncludeOp<SuperS, SubS>>,
    >,
{
    type Output = IfOp<
        SameAsOp<SuperT, SubT>,
        SetIncludeOp<SuperS, SubS>,
        AndOp<SetContainOp<SuperS, SubT>, SetIncludeOp<SuperS, SubS>>,
    >;
}

#[cfg(test)]
mod test {
    use crate::*;

    #[test]
    fn test() {
        assert_type_same!(SetContainOp<Cons<True, Nil>, False>, False);
        assert_type_same!(SetContainOp<Cons<True, Nil>, True>, True);

        assert_type_same!(
            SetIncludeOp<Cons<True, Cons<True, Nil>>, Cons<True, Nil>>,
            True
        );
        assert_type_same!(
            SetIncludeOp<Cons<True, Cons<True, Nil>>, Cons<False, Nil>>,
            False
        );
    }
}
