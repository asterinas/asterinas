pub fn reexport_fn() {
    foo::f1();
    bar::f1();
}

pub fn reexport_method() {
    let foo_struct = foo::Foo::new();
    foo_struct.get();

    let another_foo = bar::Foo::new();
    another_foo.get();
}

pub fn reexport_trait_1() {
    use foo::FooTrait;
    let foo_struct = foo::Foo::t_new();
    foo_struct.t_get();
}

pub fn reexport_trait_2() {
    use bar::FooTrait;
    let foo_struct = bar::Foo::t_new();
    foo_struct.t_get();
}