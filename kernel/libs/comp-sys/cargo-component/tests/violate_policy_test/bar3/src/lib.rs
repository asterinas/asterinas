pub static BAR: &'static usize = &foo3::FOO_ITEM;

pub fn add(left: usize, right: usize) -> usize {
    foo3::foo_add(left, right)
}