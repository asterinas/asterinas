use core::ops::Range;

/// A trait that is to ensure intervals in a set are not overlapped
pub trait NonOverlap<K> {
    /// Returns the interval
    fn range(&self) -> Range<K>;
}

/// Find all intervals that intersect with the given range.
///
/// If `NonOverlap` is implemented for the value in the set, that
/// is to say, all intervals in the set are non-overlapped, then
/// `IntersectedInterval` can be implemented for the set.
///
pub trait IntersectedInterval<'a, K: Ord + 'a, V: NonOverlap<K> + 'a> {
    type IntervalRange: IntoIterator + 'a;
    /// Find all intervals that intersect with the given range from self.
    /// Returns None if not found.
    fn intervals_for_range(&'a self, range: &Range<K>) -> Option<Self::IntervalRange>;

    /// Find all intervals that intersect with the given point from self.
    /// Returns None if not found.
    fn interval_for_point(&'a self, point: K) -> Option<&'a V>;
}
