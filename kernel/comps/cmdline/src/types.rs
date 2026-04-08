// SPDX-License-Identifier: MPL-2.0

//! Common parameter value types for the cmdline framework.
//!
//! This module provides Linux-style parsers that are frequently used by kernel
//! command lines so users of this framework don't need to rewrite them.

use alloc::vec::Vec;
use core::num::NonZeroU32;

use crate::parse::{ParamError, ParseParamValue};

/// Linux-style CPU list.
///
/// Examples:
/// - `"1"`
/// - `"1,2,10-20"`
/// - `"100-2000:2/25"` (range with stride and optional group size)
///
/// The stored representation is a list of segments; expansion is optional.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CpuList {
    segments: Vec<CpuListSegment>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CpuListSegment {
    start: u32,
    end: u32,
    /// Step within range, default 1.
    stride: NonZeroU32,
    /// Optional group size (the `/N` part). When present, stride selection is
    /// applied within each group window.
    group: Option<NonZeroU32>,
}

impl CpuList {
    pub fn segments(&self) -> &[CpuListSegment] {
        &self.segments
    }

    pub fn contains(&self, cpu: u32) -> bool {
        self.segments.iter().any(|s| segment_contains(s, cpu))
    }

    /// Expands to concrete CPU IDs, returning at most `max_elems` elements.
    ///
    /// Note: If the CPU list expands to more than `max_elems`, the result is truncated.
    pub fn expand_bounded(&self, max_elems: usize) -> Vec<u32> {
        if max_elems == 0 {
            return Vec::new();
        }

        let mut out = Vec::new();

        // Helper: Pushes a CPU ID and returns whether we reached the bound.
        let mut push_or_done_fn = |cpu: u32| -> bool {
            if out.len() >= max_elems {
                return true;
            }
            out.push(cpu);
            out.len() >= max_elems
        };

        for seg in &self.segments {
            let Some(group) = seg.group else {
                // No group. Arithmetic progression from `start` to `end` by `stride`.
                for cur in (seg.start..=seg.end).step_by(seg.stride.get() as usize) {
                    if push_or_done_fn(cur) {
                        return out;
                    }
                }
                continue;
            };

            // Grouped selection: Iterate over the groups first.
            for group_start in (seg.start..=seg.end).step_by(group.get() as usize) {
                let group_end = group_start.saturating_add(group.get() - 1).min(seg.end);

                // Arithmetic progression within the group by `stride`.
                for cur in (group_start..=group_end).step_by(seg.stride.get() as usize) {
                    if push_or_done_fn(cur) {
                        return out;
                    }
                }
            }
        }

        out
    }
}

impl CpuListSegment {
    /// Returns the start of the segment (the `N` in `N` or the `N` in `N-M`).
    pub fn start(&self) -> u32 {
        self.start
    }

    /// Returns the end of the segment (the `M` in `N-M`).
    pub fn end(&self) -> u32 {
        self.end
    }

    /// Returns the stride of the segment (the `S` in `N-M:S`).
    pub fn stride(&self) -> NonZeroU32 {
        self.stride
    }

    /// Returns the group size of the segment (the `G` in `N-M:S/G`).
    pub fn group(&self) -> Option<NonZeroU32> {
        self.group
    }
}

fn segment_contains(seg: &CpuListSegment, cpu: u32) -> bool {
    if cpu < seg.start || cpu > seg.end {
        return false;
    }

    let offset = cpu - seg.start;
    if let Some(group) = seg.group {
        let in_group_offset = offset % group.get();
        in_group_offset.is_multiple_of(seg.stride.get())
    } else {
        offset.is_multiple_of(seg.stride.get())
    }
}

impl ParseParamValue for CpuList {
    fn parse_param(value: &str) -> Result<Self, ParamError> {
        if value.is_empty() {
            return Err(ParamError::InvalidValue);
        }

        let mut segments = Vec::new();
        for part in value.split(',') {
            if part.is_empty() {
                return Err(ParamError::InvalidValue);
            }
            segments.push(parse_cpu_segment(part)?);
        }

        Ok(CpuList { segments })
    }
}

fn parse_cpu_segment(s: &str) -> Result<CpuListSegment, ParamError> {
    // Grammar (pragmatic):
    //   <range> [":" <stride> ["/" <group>] ]
    // where <range> := <n> | <n> "-" <m>
    let (range_part, tail_opt) = match s.split_once(':') {
        Some((a, b)) => (a, Some(b)),
        None => (s, None),
    };

    let (start, end) = if let Some((a, b)) = range_part.split_once('-') {
        (parse_u32(a)?, parse_u32(b)?)
    } else {
        let n = parse_u32(range_part)?;
        (n, n)
    };

    if start > end {
        return Err(ParamError::InvalidValue);
    }

    let (stride, group_opt) = match tail_opt {
        None => (1u32, None),
        Some(tail) => {
            if tail.is_empty() {
                return Err(ParamError::InvalidValue);
            }
            if let Some((stride_s, group_s)) = tail.split_once('/') {
                let stride = parse_u32(stride_s)?;
                let group = parse_u32(group_s)?;
                (stride, Some(group))
            } else {
                (parse_u32(tail)?, None)
            }
        }
    };

    let Some(stride) = NonZeroU32::new(stride) else {
        return Err(ParamError::InvalidValue);
    };

    let group = if let Some(group_val) = group_opt {
        let Some(group) = NonZeroU32::new(group_val) else {
            return Err(ParamError::InvalidValue);
        };
        Some(group)
    } else {
        None
    };

    Ok(CpuListSegment {
        start,
        end,
        stride,
        group,
    })
}

fn parse_u32(s: &str) -> Result<u32, ParamError> {
    if s.is_empty() {
        return Err(ParamError::InvalidValue);
    }
    s.parse::<u32>().map_err(|_| ParamError::InvalidValue)
}

/// Linux-style metric-suffixed u64 value.
///
/// Supports binary multiples (KiB-style):
/// - `K` = 1024
/// - `M` = 1024^2
/// - `G` = 1024^3
/// - `T` = 1024^4
/// - `P` = 1024^5
///
/// Case-insensitive.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct MetricU64(pub u64);

impl ParseParamValue for MetricU64 {
    fn parse_param(value: &str) -> Result<Self, ParamError> {
        if value.is_empty() {
            return Err(ParamError::InvalidValue);
        }

        let (num_part, suf) = match value.chars().last() {
            Some(c) if c.is_ascii_alphabetic() => (&value[..value.len() - c.len_utf8()], Some(c)),
            _ => (value, None),
        };

        if num_part.is_empty() {
            return Err(ParamError::InvalidValue);
        }

        let base: u64 = num_part.parse().map_err(|_| ParamError::InvalidValue)?;
        let mul: u64 = match suf.map(|c| c.to_ascii_uppercase()) {
            None => 1,
            Some('K') => 1024u64,
            Some('M') => 1024u64.pow(2),
            Some('G') => 1024u64.pow(3),
            Some('T') => 1024u64.pow(4),
            Some('P') => 1024u64.pow(5),
            _ => return Err(ParamError::InvalidValue),
        };

        base.checked_mul(mul)
            .map(MetricU64)
            .ok_or(ParamError::InvalidValue)
    }
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::*;

    use super::*;

    #[ktest]
    fn metric_u64_parse_ok() {
        assert_eq!(MetricU64::parse_param("0").unwrap(), MetricU64(0));
        assert_eq!(MetricU64::parse_param("1").unwrap(), MetricU64(1));

        assert_eq!(MetricU64::parse_param("1K").unwrap(), MetricU64(1024));
        assert_eq!(MetricU64::parse_param("2k").unwrap(), MetricU64(2 * 1024));

        assert_eq!(
            MetricU64::parse_param("3M").unwrap(),
            MetricU64(3 * 1024u64.pow(2))
        );
        assert_eq!(
            MetricU64::parse_param("4G").unwrap(),
            MetricU64(4 * 1024u64.pow(3))
        );
        assert_eq!(
            MetricU64::parse_param("5T").unwrap(),
            MetricU64(5 * 1024u64.pow(4))
        );
        assert_eq!(
            MetricU64::parse_param("6P").unwrap(),
            MetricU64(6 * 1024u64.pow(5))
        );
    }

    #[ktest]
    fn metric_u64_parse_err() {
        assert!(MetricU64::parse_param("").is_err());
        assert!(MetricU64::parse_param("   ").is_err());
        assert!(MetricU64::parse_param("K").is_err());
        assert!(MetricU64::parse_param("1KB").is_err());
        assert!(MetricU64::parse_param("1E").is_err());
        assert!(MetricU64::parse_param("-1").is_err());
        assert!(MetricU64::parse_param("1.5G").is_err());
    }

    #[ktest]
    fn cpu_list_parse_ok_and_contains() {
        let cl = CpuList::parse_param("1").unwrap();
        assert_eq!(cl.segments().len(), 1);
        assert!(cl.contains(1));
        assert!(!cl.contains(0));
        assert!(!cl.contains(2));

        let cl = CpuList::parse_param("1,2,10-20").unwrap();
        assert_eq!(cl.segments().len(), 3);
        assert!(cl.contains(1));
        assert!(cl.contains(2));
        assert!(cl.contains(10));
        assert!(cl.contains(20));
        assert!(!cl.contains(21));

        // range with stride: 100-110:2 => 100,102,104,106,108,110
        let cl = CpuList::parse_param("100-110:2").unwrap();
        assert!(cl.contains(100));
        assert!(!cl.contains(101));
        assert!(cl.contains(102));
        assert!(!cl.contains(103));
        assert!(cl.contains(110));
    }

    #[ktest]
    fn cpu_list_parse_ok_group_stride() {
        // 0-9:2/4
        // windows of 4: [0..3] picks 0,2; [4..7] picks 4,6; [8..11] picks 8,10
        // intersect with 0..9 => 0,2,4,6,8
        let cl = CpuList::parse_param("0-9:2/4").unwrap();
        assert!(cl.contains(0));
        assert!(!cl.contains(1));
        assert!(cl.contains(2));
        assert!(!cl.contains(3));
        assert!(cl.contains(4));
        assert!(!cl.contains(5));
        assert!(cl.contains(6));
        assert!(!cl.contains(7));
        assert!(cl.contains(8));
        assert!(!cl.contains(9));
    }

    #[ktest]
    fn cpu_list_parse_err() {
        assert!(CpuList::parse_param("").is_err());
        assert!(CpuList::parse_param("   ").is_err());
        assert!(CpuList::parse_param(",").is_err());
        assert!(CpuList::parse_param("1,").is_err());
        assert!(CpuList::parse_param("1,,2").is_err());
        assert!(CpuList::parse_param("a").is_err());
        assert!(CpuList::parse_param("1-a").is_err());
        assert!(CpuList::parse_param("2-1").is_err()); // start > end
        assert!(CpuList::parse_param("1:0").is_err()); // stride 0
        assert!(CpuList::parse_param("1:2/0").is_err()); // group 0
        assert!(CpuList::parse_param("1:").is_err());
        assert!(CpuList::parse_param("1-/2").is_err());
    }

    #[ktest]
    fn cpu_list_expand_bounded() {
        let cl = CpuList::parse_param("10-20:2").unwrap(); // 10,12,14,16,18,20 => 6 elems
        let v = cl.expand_bounded(6);
        assert_eq!(v.as_slice(), &[10u32, 12u32, 14u32, 16u32, 18u32, 20u32]);
        let v = cl.expand_bounded(5);
        assert_eq!(v.as_slice(), &[10u32, 12u32, 14u32, 16u32, 18u32]);
    }

    #[ktest]
    fn cpu_list_expand_bounded_large_range_stride_fast() {
        // 0-4_000_000_000:1024 should not scan the whole range; it should just step by 1024.
        // We only ask for a few elements to ensure it returns quickly and correctly.
        let cl = CpuList::parse_param("0-4000000000:1024").unwrap();
        let v = cl.expand_bounded(4);
        assert_eq!(v.as_slice(), &[0u32, 1024u32, 2048u32, 3072u32]);
    }

    #[ktest]
    fn cpu_list_expand_bounded_group_large_range_fast() {
        // Grouped selection still shouldn't scan by +1.
        // 0-4_000_000_000:2/4 => in each group of 4 select offsets 0 and 2.
        // First few selected: 0,2,4,6,8,10,...
        let cl = CpuList::parse_param("0-4000000000:2/4").unwrap();
        let v = cl.expand_bounded(6);
        assert_eq!(v.as_slice(), &[0u32, 2u32, 4u32, 6u32, 8u32, 10u32]);
    }

    #[ktest]
    fn cpu_list_expand_bounded_group_respects_end_boundary() {
        // Ensure the last (partial) group is clipped by end.
        // 0-5:2/4 => groups [0..3] => 0,2; [4..7] => 4,6 but end=5 so only 4.
        let cl = CpuList::parse_param("0-5:2/4").unwrap();
        let v = cl.expand_bounded(8);
        assert_eq!(v.as_slice(), &[0u32, 2u32, 4u32]);
    }

    #[ktest]
    fn cpu_list_expand_bounded_group_stride_gt_group() {
        // stride > group: only within=0 is emitted per group.
        // 0-20:8/4 => groups start at 0,4,8,12,16,20 => emit 0,4,8,12,16,20
        let cl = CpuList::parse_param("0-20:8/4").unwrap();
        let v = cl.expand_bounded(16);
        assert_eq!(v.as_slice(), &[0u32, 4u32, 8u32, 12u32, 16u32, 20u32]);
    }
}
