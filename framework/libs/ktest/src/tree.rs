//! The source module tree of ktests.
//!
//! In the `KtestTree`, the root is abstract, and the children of the root are the
//! crates. The leaves are the test functions. Nodes other than the root and the
//! leaves are modules.
//!

use alloc::{
    collections::{btree_map, BTreeMap},
    string::{String, ToString},
    vec,
    vec::Vec,
};
use core::iter::{FromIterator, Iterator};

use crate::{
    path::{KtestPath, PathElement},
    KtestItem,
};

#[derive(Debug)]
pub struct KtestModule {
    nr_tot_tests: usize,
    name: PathElement,
    children: BTreeMap<PathElement, KtestModule>,
    tests: Vec<KtestItem>,
}

impl KtestModule {
    pub fn nr_this_tests(&self) -> usize {
        self.tests.len()
    }

    pub fn nr_tot_tests(&self) -> usize {
        self.nr_tot_tests
    }

    pub fn name(&self) -> &PathElement {
        &self.name
    }

    fn insert(&mut self, module_path: &mut KtestPath, test: KtestItem) {
        self.nr_tot_tests += 1;
        if module_path.is_empty() {
            self.tests.push(test);
        } else {
            let module_name = module_path.pop_front().unwrap();
            let node = self.children.entry(module_name.clone()).or_insert(Self {
                nr_tot_tests: 0,
                name: module_name,
                children: BTreeMap::new(),
                tests: Vec::new(),
            });
            node.nr_tot_tests += 1;
            node.insert(module_path, test);
        }
    }

    fn new(name: PathElement) -> Self {
        Self {
            nr_tot_tests: 0,
            name,
            children: BTreeMap::new(),
            tests: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct KtestCrate {
    // Crate behaves just like modules, which can own it's children and tests.
    // But the iterator it provides will only iterate over the modules not tests.
    root_module: KtestModule,
}

impl KtestCrate {
    pub fn nr_tot_tests(&self) -> usize {
        self.root_module.nr_tot_tests()
    }

    pub fn name(&self) -> &str {
        self.root_module.name()
    }
}

pub struct KtestTree {
    nr_tot_tests: usize,
    crates: BTreeMap<String, KtestCrate>,
}

impl FromIterator<KtestItem> for KtestTree {
    fn from_iter<I: IntoIterator<Item = KtestItem>>(iter: I) -> Self {
        let mut tree = Self::new();
        for test in iter {
            tree.add_ktest(test);
        }
        tree
    }
}

impl KtestTree {
    pub fn new() -> Self {
        Self {
            nr_tot_tests: 0,
            crates: BTreeMap::new(),
        }
    }

    pub fn add_ktest(&mut self, test: KtestItem) {
        self.nr_tot_tests += 1;
        let package = test.info().package.to_string();
        let module_path = test.info().module_path;
        let node = self.crates.entry(package.clone()).or_insert(KtestCrate {
            root_module: KtestModule::new(PathElement::from(package)),
        });
        node.root_module
            .insert(&mut KtestPath::from(module_path), test);
    }

    pub fn nr_tot_tests(&self) -> usize {
        self.nr_tot_tests
    }

    pub fn nr_tot_crates(&self) -> usize {
        self.crates.len()
    }
}

impl Default for KtestTree {
    fn default() -> Self {
        Self::new()
    }
}

/// The `KtestTreeIter` will iterate over all crates. Yeilding `KtestCrate`s.
pub struct KtestTreeIter<'a> {
    crate_iter: btree_map::Iter<'a, String, KtestCrate>,
}

impl KtestTree {
    pub fn iter(&self) -> KtestTreeIter<'_> {
        KtestTreeIter {
            crate_iter: self.crates.iter(),
        }
    }
}

impl<'a> Iterator for KtestTreeIter<'a> {
    type Item = &'a KtestCrate;

    fn next(&mut self) -> Option<Self::Item> {
        self.crate_iter.next().map(|(_, v)| v)
    }
}

type CrateChildrenIter<'a> = btree_map::Iter<'a, String, KtestModule>;

/// The `KtestCrateIter` will iterate over all modules in a crate. Yeilding `KtestModule`s.
/// The iterator will return modules in the depth-first-search order of the module tree.
pub struct KtestCrateIter<'a> {
    path: Vec<(&'a KtestModule, CrateChildrenIter<'a>)>,
}

impl KtestCrate {
    pub fn iter(&self) -> KtestCrateIter<'_> {
        KtestCrateIter {
            path: vec![(&self.root_module, self.root_module.children.iter())],
        }
    }
}

impl<'a> Iterator for KtestCrateIter<'a> {
    type Item = &'a KtestModule;

    fn next(&mut self) -> Option<Self::Item> {
        let next_module = loop {
            let Some(last) = self.path.last_mut() else {
                break None;
            };
            if let Some((_, next_module)) = last.1.next() {
                break Some(next_module);
            }
            let (_, _) = self.path.pop().unwrap();
        };
        if let Some(next_module) = next_module {
            self.path.push((next_module, next_module.children.iter()));
            Some(next_module)
        } else {
            None
        }
    }
}

/// The `KtestModuleIter` will iterate over all tests in a crate. Yeilding `KtestItem`s.
pub struct KtestModuleIter<'a> {
    test_iter: core::slice::Iter<'a, KtestItem>,
}

impl KtestModule {
    pub fn iter(&self) -> KtestModuleIter<'_> {
        KtestModuleIter {
            test_iter: self.tests.iter(),
        }
    }
}

impl<'a> Iterator for KtestModuleIter<'a> {
    type Item = &'a KtestItem;

    fn next(&mut self) -> Option<Self::Item> {
        self.test_iter.next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! gen_test_case {
        () => {{
            fn dummy_fn() {
                ()
            }
            let mut tree = KtestTree::new();
            let new = |m: &'static str, f: &'static str, p: &'static str| {
                KtestItem::new(
                    dummy_fn,
                    (false, None),
                    crate::KtestItemInfo {
                        module_path: m,
                        fn_name: f,
                        package: p,
                        source: "unrelated",
                        line: 0,
                        col: 0,
                    },
                )
            };
            tree.add_ktest(new("crate1::mod1::mod2", "test21", "crate1"));
            tree.add_ktest(new("crate1::mod1", "test11", "crate1"));
            tree.add_ktest(new("crate1::mod1::mod2", "test22", "crate1"));
            tree.add_ktest(new("crate1::mod1::mod2", "test23", "crate1"));
            tree.add_ktest(new("crate1::mod1::mod3", "test31", "crate1"));
            tree.add_ktest(new("crate1::mod1::mod3::mod4", "test41", "crate1"));
            tree.add_ktest(new("crate2::mod1::mod2", "test2", "crate2"));
            tree.add_ktest(new("crate2::mod1", "test1", "crate2"));
            tree.add_ktest(new("crate2::mod1::mod2", "test3", "crate2"));
            tree
        }};
    }

    #[test]
    fn test_tree_iter() {
        let tree = gen_test_case!();
        let mut iter = tree.iter();
        let c1 = iter.next().unwrap();
        assert_eq!(c1.name(), "crate1");
        let c2 = iter.next().unwrap();
        assert_eq!(c2.name(), "crate2");
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_crate_iter() {
        let tree = gen_test_case!();
        for crate_ in tree.iter() {
            if crate_.name() == "crate1" {
                let mut len = 0;
                for module in crate_.iter() {
                    len += 1;
                    let modules = ["crate1", "mod1", "mod2", "mod3", "mod4"];
                    assert!(modules.contains(&module.name().as_str()));
                }
                assert_eq!(len, 5);
            } else if crate_.name() == "crate2" {
                let mut len = 0;
                for module in crate_.iter() {
                    len += 1;
                    let modules = ["crate2", "mod1", "mod2"];
                    assert!(modules.contains(&module.name().as_str()));
                }
                assert_eq!(len, 3);
            }
        }
    }

    #[test]
    fn test_module_iter() {
        let tree = gen_test_case!();
        let mut collection = Vec::<&KtestItem>::new();
        for crate_ in tree.iter() {
            for mov in crate_.iter() {
                let module = mov;
                for test in module.iter() {
                    collection.push(&test);
                }
            }
        }
        assert_eq!(collection.len(), 9);
        assert!(collection.iter().any(|t| t.info().fn_name == "test1"));
        assert!(collection.iter().any(|t| t.info().fn_name == "test2"));
        assert!(collection.iter().any(|t| t.info().fn_name == "test3"));
        assert!(collection.iter().any(|t| t.info().fn_name == "test11"));
        assert!(collection.iter().any(|t| t.info().fn_name == "test21"));
        assert!(collection.iter().any(|t| t.info().fn_name == "test22"));
        assert!(collection.iter().any(|t| t.info().fn_name == "test23"));
        assert!(collection.iter().any(|t| t.info().fn_name == "test31"));
        assert!(collection.iter().any(|t| t.info().fn_name == "test41"));
    }
}
