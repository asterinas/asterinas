#![feature(register_tool)]
#![register_tool(component_access_control)]

#[macro_use]
extern crate controlled;

#[controlled]
pub fn f1() {}

pub struct Foo;

impl Foo {
    #[controlled]
    pub fn new() -> Self {
        Foo
    }

    #[controlled]
    pub fn get(&self) -> usize {
        42
    }
}

pub trait FooTrait {
    #[controlled]
    fn t_new() -> Self;
    #[controlled]
    fn t_get(&self) -> usize;
}

impl FooTrait for Foo {
    fn t_new() -> Self {
        Foo
    }

    fn t_get(&self) -> usize {
        43
    }
}