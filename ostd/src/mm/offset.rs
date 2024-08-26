// SPDX-License-Identifier: MPL-2.0

/// Gets the offset of a field within a type as a pointer.
///
/// ```rust
/// #[repr(C)]
/// pub struct Foo {
///     first: u8,
///     second: u32,
/// }
///
/// assert!(offset_of(Foo, first) == (0 as *const u8));
/// assert!(offset_of(Foo, second) == (4 as *const u32));
/// ```
#[macro_export]
macro_rules! offset_of {
    ($container:ty, $($field:tt)+) => ({
        // SAFETY: It is ok to have this uninitialized value because
        // 1) Its memory won't be accessed;
        // 2) It will be forgotten rather than being dropped;
        // 3) Before it gets forgotten, the code won't return prematurely or panic.
        let tmp: $container = unsafe { core::mem::MaybeUninit::uninit().assume_init() };

        let container_addr = &tmp as *const _;
        let field_addr =  &tmp.$($field)* as *const _;

        ::core::mem::forget(tmp);

        let field_offset = (field_addr as usize - container_addr as usize) as *const _;

        // Let Rust compiler infer our intended pointer type of field_offset
        // by comparing it with another pointer.
        let _: bool = field_offset == field_addr;

        field_offset
    });
}

/// Gets the offset of a field within an object as a pointer.
///
/// ```rust
/// #[repr(C)]
/// pub struct Foo {
///     first: u8,
///     second: u32,
/// }
/// let foo = &Foo {first: 0, second: 0};
/// assert!(value_offset!(foo) == (0 as *const Foo));
/// assert!(value_offset!(foo.first) == (0 as *const u8));
/// assert!(value_offset!(foo.second) == (4 as *const u32));
/// ```
#[macro_export]
macro_rules! value_offset {
    ($container:ident) => ({
        let container_addr = &*$container as *const _;
        let offset = 0 as *const _;
        let _: bool = offset == container_addr;
        offset
    });
    ($container:ident.$($field:ident).*) => ({
        let container_addr = &*$container as *const _;
        // SAFETY: This is safe since we never access the field
        let field_addr = unsafe {&($container.$($field).*)} as *const _;
        let field_offset = (field_addr as usize- container_addr as usize) as *const _;
        let _: bool = field_offset == field_addr;
        field_offset
    });
}
