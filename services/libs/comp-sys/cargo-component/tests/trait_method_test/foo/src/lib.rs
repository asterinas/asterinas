#![feature(register_tool)]
#![register_tool(component_access_control)]

#[macro_use]
extern crate controlled;

pub struct Foo;

impl Foo {
    #[controlled]
    pub fn associate_fn() -> Self {
        todo!()
    }

    #[controlled]
    pub fn method(&self) -> usize {
        todo!()
    }
}

pub trait FooTrait {
    #[controlled]
    fn trait_associate_fn();

    #[controlled]
    fn trait_method(&self) -> usize;
}

impl FooTrait for Foo {
    fn trait_associate_fn() {
        todo!()
    }

    fn trait_method(&self) -> usize {
        todo!()
    }
}

pub trait ObjectSafeTrait {
    #[controlled]
    fn get(&self) -> usize;
}

impl ObjectSafeTrait for Foo {
    fn get(&self) -> usize {
        todo!()
    }
}
