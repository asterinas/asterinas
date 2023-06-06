use core::marker::PhantomData;

use crate::assert_type_same;

pub trait TypeWrap {
    type WrappedType;
}

pub type TypeWrapOp<T> = <T as TypeWrap>::WrappedType;

#[derive(Clone, Copy)]
pub struct TypeWrapper<T>(pub PhantomData<T>);

impl<T> TypeWrapper<T> {
    pub fn new_default() -> Self {
        Self(PhantomData)
    }
}

impl<T> TypeWrap for TypeWrapper<T> {
    type WrappedType = T;
}

assert_type_same!(u16, TypeWrapOp<TypeWrapper<u16>>);
