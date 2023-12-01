use alloc::{
    collections::{vec_deque, BTreeMap, VecDeque},
    string::{String, ToString},
};
use core::{fmt::Display, iter::zip, ops::Deref};

pub type PathElement = String;

pub type KtestPathIter<'a> = vec_deque::Iter<'a, PathElement>;

#[derive(Debug)]
pub struct KtestPath {
    path: VecDeque<PathElement>,
}

impl From<&str> for KtestPath {
    fn from(s: &str) -> Self {
        let mut path = VecDeque::new();
        for module in s.split("::") {
            path.push_back(module.to_string());
        }
        Self { path }
    }
}

impl KtestPath {
    pub fn new() -> Self {
        Self {
            path: VecDeque::new(),
        }
    }

    pub fn from(s: &str) -> Self {
        Self {
            path: s.split("::").map(PathElement::from).collect(),
        }
    }

    pub fn push_back(&mut self, s: &str) {
        self.path.push_back(PathElement::from(s));
    }

    pub fn pop_back(&mut self) -> Option<PathElement> {
        self.path.pop_back()
    }

    pub fn push_front(&mut self, s: &str) {
        self.path.push_front(PathElement::from(s))
    }

    pub fn pop_front(&mut self) -> Option<PathElement> {
        self.path.pop_front()
    }

    pub fn len(&self) -> usize {
        self.path.len()
    }

    pub fn is_empty(&self) -> bool {
        self.path.is_empty()
    }

    pub fn starts_with(&self, other: &Self) -> bool {
        if self.path.len() < other.path.len() {
            return false;
        }
        for (e1, e2) in zip(self.path.iter(), other.path.iter()) {
            if e1 != e2 {
                return false;
            }
        }
        true
    }

    pub fn ends_with(&self, other: &Self) -> bool {
        if self.path.len() < other.path.len() {
            return false;
        }
        for (e1, e2) in zip(self.path.iter().rev(), other.path.iter().rev()) {
            if e1 != e2 {
                return false;
            }
        }
        true
    }

    pub fn iter(&self) -> KtestPathIter {
        self.path.iter()
    }
}

impl Default for KtestPath {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for KtestPath {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        let mut first = true;
        for e in self.path.iter() {
            if first {
                first = false;
            } else {
                write!(f, "::")?;
            }
            write!(f, "{}", e)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod path_test {
    use super::*;

    #[test]
    fn test_ktest_path() {
        let mut path = KtestPath::new();
        path.push_back("a");
        path.push_back("b");
        path.push_back("c");
        assert_eq!(path.to_string(), "a::b::c");
        assert_eq!(path.pop_back(), Some("c".to_string()));
        assert_eq!(path.pop_back(), Some("b".to_string()));
        assert_eq!(path.pop_back(), Some("a".to_string()));
        assert_eq!(path.pop_back(), None);
    }

    #[test]
    fn test_ktest_path_starts_with() {
        let mut path = KtestPath::new();
        path.push_back("a");
        path.push_back("b");
        path.push_back("c");
        assert!(path.starts_with(&KtestPath::from("a")));
        assert!(path.starts_with(&KtestPath::from("a::b")));
        assert!(path.starts_with(&KtestPath::from("a::b::c")));
        assert!(!path.starts_with(&KtestPath::from("a::b::c::d")));
        assert!(!path.starts_with(&KtestPath::from("a::b::d")));
        assert!(!path.starts_with(&KtestPath::from("a::d")));
        assert!(!path.starts_with(&KtestPath::from("d")));
    }

    #[test]
    fn test_ktest_path_ends_with() {
        let mut path = KtestPath::new();
        path.push_back("a");
        path.push_back("b");
        path.push_back("c");
        assert!(path.ends_with(&KtestPath::from("c")));
        assert!(path.ends_with(&KtestPath::from("b::c")));
        assert!(path.ends_with(&KtestPath::from("a::b::c")));
        assert!(!path.ends_with(&KtestPath::from("d::a::b::c")));
        assert!(!path.ends_with(&KtestPath::from("a::b::d")));
        assert!(!path.ends_with(&KtestPath::from("a::d")));
        assert!(!path.ends_with(&KtestPath::from("d")));
    }
}

#[derive(Debug)]
pub struct SuffixTrie {
    children: BTreeMap<PathElement, SuffixTrie>,
    is_end: bool,
}

impl SuffixTrie {
    pub fn new() -> Self {
        Self {
            children: BTreeMap::new(),
            is_end: false,
        }
    }

    pub fn from_paths<I: IntoIterator<Item = KtestPath>>(paths: I) -> Self {
        let mut t = Self::new();
        for i in paths {
            t.insert(i.iter());
        }
        t
    }

    pub fn insert<I, P>(&mut self, path: I)
    where
        I: DoubleEndedIterator<Item = P>,
        P: Deref<Target = PathElement>,
    {
        let mut cur = self;
        for e in path.into_iter().rev() {
            cur = cur.children.entry(e.clone()).or_default();
        }
        cur.is_end = true;
    }

    /// Find if there is a perfect match in this suffix trie.
    pub fn matches<I, P>(&self, path: I) -> bool
    where
        I: DoubleEndedIterator<Item = P>,
        P: Deref<Target = PathElement>,
    {
        let mut cur = self;
        for e in path.into_iter().rev() {
            if let Some(next) = cur.children.get(&*e) {
                cur = next;
            } else {
                return false;
            }
        }
        cur.is_end
    }

    /// Find if any suffix of the path exists in the suffix trie.
    pub fn contains<I, P>(&self, path: I) -> bool
    where
        I: DoubleEndedIterator<Item = P>,
        P: Deref<Target = PathElement>,
    {
        let mut cur = self;
        for e in path.into_iter().rev() {
            if let Some(next) = cur.children.get(&*e) {
                cur = next;
                if cur.is_end {
                    return true;
                }
            } else {
                return false;
            }
        }
        false
    }
}

impl Default for SuffixTrie {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod suffix_trie_test {
    use super::*;

    static TEST_PATHS: &[&str] = &[
        "a::b::c::d",
        "a::b::c::e",
        "a::b::d::e",
        "a::b::f::g",
        "h::i::j::k",
        "l::m::n",
        "m::n",
    ];

    #[test]
    fn test_contains() {
        let trie = SuffixTrie::from_paths(TEST_PATHS.iter().map(|&s| KtestPath::from(s)));

        assert!(trie.contains(KtestPath::from("e::f::g::a::b::c::d").iter()));
        assert!(trie.contains(KtestPath::from("e::f::g::a::b::f::g").iter()));
        assert!(trie.contains(KtestPath::from("h::i::j::l::m::n").iter()));
        assert!(trie.contains(KtestPath::from("l::m::n").iter()));

        assert!(!trie.contains(KtestPath::from("a::b::c").iter()));
        assert!(!trie.contains(KtestPath::from("b::c::d").iter()));
        assert!(!trie.contains(KtestPath::from("a::b::f").iter()));
        assert!(!trie.contains(KtestPath::from("i::j").iter()));
        assert!(!trie.contains(KtestPath::from("h::i::j::l::n").iter()));
        assert!(!trie.contains(KtestPath::from("n").iter()));
    }

    #[test]
    fn test_matches() {
        let trie = SuffixTrie::from_paths(TEST_PATHS.iter().map(|&s| KtestPath::from(s)));

        assert!(trie.matches(KtestPath::from("a::b::c::d").iter()));
        assert!(trie.matches(KtestPath::from("a::b::c::e").iter()));
        assert!(trie.matches(KtestPath::from("l::m::n").iter()));
        assert!(trie.matches(KtestPath::from("m::n").iter()));

        assert!(!trie.matches(KtestPath::from("a::b::d").iter()));
        assert!(!trie.matches(KtestPath::from("b::c::e").iter()));
        assert!(!trie.matches(KtestPath::from("l::n::h::k::j::i").iter()));
        assert!(!trie.matches(KtestPath::from("n").iter()));
    }
}
