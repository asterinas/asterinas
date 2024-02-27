use foo2::{ObjectSafeTrait, Foo, FooTrait};

pub fn method() {
    let foo_struct = Foo::associate_fn();
    let val = foo_struct.method();
    println!("val = {}", val);
}

pub fn trait_method() {
    Foo::trait_associate_fn();
    let foo_struct = Foo::associate_fn();
    foo_struct.trait_method();
}

pub fn dyn_trait() {
    let foo_as_dyn = Box::new(Foo::associate_fn()) as Box<dyn ObjectSafeTrait>;
    foo_as_dyn.get();
}

pub fn opaque_type(object: impl ObjectSafeTrait) {
    object.get();
}

pub fn generic<T: ObjectSafeTrait>(object: T) {
    object.get();
}
