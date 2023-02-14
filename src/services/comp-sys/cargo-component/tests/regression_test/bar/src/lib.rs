pub static BAR: &'static usize = &foo::FOO_ITEM;

pub fn add(left: usize, right: usize) -> usize {
    foo::foo_add(left, right)
}
