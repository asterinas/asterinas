// SPDX-License-Identifier: MPL-2.0

use core::marker::PhantomData;

use ostd::Pod;

/// This ptr is designed to read union field safely.
/// Write to union field is safe operation. While reading union field is UB.
/// However, if this field is Pod, we can safely read that field.
pub struct UnionReadPtr<'a, T: Pod> {
    bytes: &'a [u8],
    marker: PhantomData<&'a mut T>,
}

impl<'a, T: Pod> UnionReadPtr<'a, T> {
    pub fn new(object: &'a T) -> Self {
        let bytes = object.as_bytes();
        Self {
            bytes,
            marker: PhantomData,
        }
    }

    pub fn read_at<F: Pod>(&self, offset: *const F) -> F {
        let offset = offset as usize;
        F::from_bytes(&self.bytes[offset..])
    }
}

/// FIXME: This macro requires the first argument to be a `reference` of a Pod type.
/// This is because the value_offset macro requires
#[macro_export]
macro_rules! read_union_fields {
    ($container:ident) => ({
        let offset = value_offset!($container);
        let union_read_ptr = UnionReadPtr::new(&*$container);
        union_read_ptr.read_at(offset)
    });
    ($container:ident.$($field:ident).*) => ({
        let field_offset = ostd::value_offset!($container.$($field).*);
        let union_read_ptr = UnionReadPtr::new(&*$container);
        union_read_ptr.read_at(field_offset)
    });
}
