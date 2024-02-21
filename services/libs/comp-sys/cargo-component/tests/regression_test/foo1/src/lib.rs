#![feature(register_tool)]
#![register_tool(component_access_control)]

#[macro_use]
extern crate controlled;

#[controlled]
pub static FOO_ITEM: usize = 0;

#[controlled]
pub fn foo_add(left: usize, right: usize) -> usize {
    left + right
}
