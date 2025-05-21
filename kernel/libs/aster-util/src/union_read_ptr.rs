// SPDX-License-Identifier: MPL-2.0

use ostd::Pod;

/// A reader to read `Pod` fields from a `Pod` type.
pub struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn new<T: Pod>(object: &'a T) -> Self {
        Self {
            bytes: object.as_bytes(),
        }
    }

    pub fn read_at<F: Pod>(&self, field_offset: usize, _type_infer: *const F) -> F {
        F::from_bytes(&self.bytes[field_offset..])
    }
}

#[macro_export]
macro_rules! read_union_field {
    ($container:expr, $type:ty, $($field:tt)+) => {{
        use $crate::union_read_ptr::Reader;

        // Perform type checking first.
        let container: &$type = $container;
        let reader = Reader::new(container);

        let field_offset = core::mem::offset_of!($type, $($field)*);
        let type_infer = ostd::ptr_null_of!({
            // This is not safe, but the code won't be executed.
            &raw const container.$($field)*
        });

        reader.read_at(field_offset, type_infer)
    }}
}
