# RFC-0003: CortenMM

* Status: Draft
* Pull request: https://github.com/asterinas/asterinas/pull/2792
* Date submitted: 2025-12-24
* Date approved: YYYY-MM-DD

## Summary

This RFC introduces CortenMM[^1]’s single-level, page-table–centric virtual memory management in Asterinas. Instead of maintaining a separate `VmMapping` tree plus page tables, we store mapping metadata alongside page tables and split mappings on page-table boundaries. This yields better scalability, simpler invariants, and stronger correctness properties while preserving Linux-compatible semantics.

## Motivation

Two-level VM designs (e.g., VMA trees and page tables) hurt scalability and are a recurring source of subtle concurrency bugs; Linux continues to carry `mmap_lock`/VMA contention and correctness CVEs. Our paper, CortenMM, addresses these issues and has gained substantial positive feedback from the OS research community.

The CortenMM paper shows that mainstream ISAs all use radix page tables, so we can collapse to a single level without losing portability. By colocating mapping metadata with page tables, we eliminate cross-structure synchronization and reduce cache/lock traffic. The paper reports 1.2×–26× speedups on real workloads; we expect similar gains for Asterinas user space and kernel services.

A single-level abstraction simplifies reasoning and enables stronger correctness: invariants are local to page-table subtrees; our merged locking protocol already enforces serialized updates per covered range. This reduces risk as we continue to develop for production and broaden hardware coverage.

## Design

This RFC proposes a full replacement of the existing two-level VM system with CortenMM’s single-level design. The design largely follows the CortenMM paper but uses a slightly different metadata storage strategy for compatibility and memory-efficiency reasons.

### High-level shape

The page table is the source of truth; per-page-table auxiliary metadata stores `VmMapping`s that cover that page table frame's range. No global VMA tree is required.

`VmMapping`s are split at page-table boundaries when inserted and each PT node only carries mappings aligned to its range. This keeps lookups and updates local and mergeable.

For example, this is the page table after `mmap(offset=0x3fe000, size=0x604000)`. A new $2 \text{MiB} \times 3 + 4 \text{KiB} \times 4$ `VmMapping` is split into 3 pieces (`[x]`, `[y]` and `[z]`) and stored across 2 levels. There are also two placeholders (`[i]` and `[j]`) indicating the presence of child page tables:

```
                   .----------------.
L4                 |      #4-1      |
                   '.---------------'
                   /
                  v
                 .----------------.
L3               |      #3-1      |
                 '.---------------'
                 /
                v
               .----------------.
L2             |      #2-1      |
               '-.---------.----'
      .--------[i]   [y]   [j]
      v                     v
     .----------------.     .----------------.
L1   |      #1-2      |     |      #1-6      |
     '----------------'     '----------------'
                    [x]      [z]

Pieces stored:
    0x3fe000       0x400000               0xa00000      0xa02000
    [      x      ][          y          ][     z      ]
      at #1-2 PT         at #2-1 PT         at #1-6 PT

[        i        ]                       [        j        ]
    at #2-1 PT                                at #2-1 PT
```

If a page fault happens at `0x600000`, the handler locks the `#2-1` PT node, splits `[y]` into `[a]`, `[b]`, and `[c]` at the `#1-4` PT boundary, maps the page, and inserts `[l]` as a placeholder indicating the presence of a child page table:

```
                   .----------------.
L4                 |      #4-1      |
                   '.---------------'
                   /
                  v
                 .----------------.
L3               |      #3-1      |
                 '.---------------'
                 /
                v
               .----------------.
L2             |      #2-1      |
               '-.----.----.----'
      .--------[i][a][l][c][j]---------------.
      v               '--v                   v
     .----------------.  .----------------.  .----------------.
L1   |      #1-2      |  |      #1-4      |  |      #1-6      |
     '----------------'  '----------------'  '----------------'
                    [x]  [        b       ]  [z]

Pieces stored:
   0x3fe000    0x400000        0x600000        0x800000        0xa00000    0xa02000
   [     x    ][       a      ][       b      ][      c       ][     z    ]
    at #1-2 PT    at #2-1 PT      at #1-4 PT      at #2-1 PT    at #1-6 PT

[      i      ]                [       l      ]                [      j      ]
   at #2-1 PT                     at #2-1 PT                     at #2-1 PT
```

Within each PT node, the `VmMapping`s are stored in an interval set (e.g., a `BTreeMap` keyed by range) to support efficient lookups, splits, merges, and removals.

The page table locking protocol described in the paper is already present in Asterinas (cursor-based per-range locking over PT pages). This RFC focuses on the data-structure shift and metadata placement; the locking protocol remains unchanged.

Upper kernel components access the page table via the same OSTD cursor APIs as before, with minor adjustments to define and access metadata in PT-local storage.

Other optimizations introduced in the paper for full scalability (LATR[^4], MCS locks) are out of scope for this RFC but can be pursued later.

The following sections describe the key implementations of existing memory management system calls.

### Address space allocation for `mmap`/`mremap`

There are two kinds of `mmap`/`mremap` calls, depending on the `MAP_FIXED`/`MREMAP_FIXED` flag. Without this flag, non-fixed `mmap`/`mremap` allocates a new address space range. With this flag, fixed `mmap`/`mremap` overwrites an existing address space range.

According to the CortenMM paper, an operation starts with a cursor that locks the given range. However, non-fixed `mmap`/`mremap` does not know the range yet, so it must find a free range first. To do this, non-fixed `mmap`/`mremap` first locks a large range with sized $\dfrac{1}{N}$ (where $N$ is the number of CPUs) and tries to find a free range inside it according to the auxiliary metadata stored in the page tables. If it fails, it unlocks the range and retries with a different range (or the entire address space) until it finds a free range. On the other hand, fixed `mmap`/`mremap` directly locks the given range.

With this design, both fixed and non-fixed `mmap`/`mremap` can be implemented scalably with the same cursor-based APIs.

### Traversing the page table

With auxiliary metadata, page-table traversal is more efficient. We can provide APIs that consult the [`VmMapping`] set rather than the page-table entries, enabling faster `fork`s.

### New OSTD APIs

OSTD must not be aware of the auxiliary metadata type. We introduce a new `AuxPageTableMeta` trait that allows users to define and access auxiliary metadata in page-table nodes. The `VmMapping` set is defined as the auxiliary metadata type. The trait is:

```rust
/// Auxiliary metadata for user page tables.
pub trait AuxPageTableMeta: AuxPtMetaLayoutChecked + Debug + Send + Sync + 'static {
    /// The callback to allocate a new root page table.
    fn new_root_page_table() -> Self;

    /// The callback to allocate a child page table.
    ///
    /// It is called when a new page table is allocated under the page table
    /// entry at virtual address `va` and at level `level`. A new page table
    /// will be allocated when:
    ///  - preparing a new page table for mapping in lower levels;
    ///  - splitting a huge mapping into smaller mappings.
    ///
    /// The receiver [`AuxPageTableMeta`] is the metadata of the parent page
    /// table and the returned [`AuxPageTableMeta`] will be the metadata of
    /// the allocated child page table.
    fn alloc_child_page_table(&mut self, va: Vaddr, level: PagingLevel) -> Self;
}
```

The user-defined metadata type will be provided to OSTD via `VmSpace`'s new generic parameter `VmSpace<A: AuxPageTableMeta>`. The `()` type also implements the `AuxPageTableMeta` trait as a no-op implementation for backward compatibility. The `VmSpace` will instantiate page table with the `Aux` type in the `PageTableConfig` trait specified as the user-defined type.

The existing OSTD cursor APIs need to support the auxiliary metadata. Therefore, the cursor user must be aware of the level of the cursor, rather than only the virtual address.

## Drawbacks, Alternatives, and Unknown

**Drawbacks:**
 - Slightly higher metadata footprint when mappings straddle many PT boundaries (mitigated by merging to upper-level page table frames that manages larger ranges).
 - Portability to non-radix MMUs would require a different scheme; this RFC targets x86_64/RISC-V/ARM.
 - Imprecise `RLIMIT_AS`. To make `mmap` scalable, the total size of virtual address mappings is implemented with per-CPU counters. So the user program may not immediately abort if it exceeds the `RLIMIT_AS` limit.

**Alternatives:**
 - Keep the existing two-level design and place CortenMM under a feature gate `#[cfg(feature = "cortenmm")]`. This would allow opt-in testing but complicates maintenance and feature development.
 - Split the virtual address space into two regions: one for two-level VM and one for CortenMM. And allows run-time selection of VM management. This yields best compatibility but adds complexity and fragmentation. It would be a useful stepping stone for implementing CortenMM in Linux. But for Asterinas, we do not have existing drivers and modules relying on the two-level design, so the added complexity may not be justified.
 - Keep two-level design but optimize VMA management (e.g., via RCU-safe maple trees[^3] or concurrent skip lists[^2]). This would improve scalability but not eliminate cross-structure synchronization or correctness risks. These scalable data structures are also complex and require `unsafe` code, which bloats the TCB and increases maintenance burden.

**Unresolved Questions / follow-ups:**
 - Asterinas does not feature reverse-mapping at the moment. CortenMM's reverse-mapping design is currently pre-mature and out of scope for this RFC.

## Prior Art and References

[RadixVM: scalable address spaces for multithreaded applications](https://dl.acm.org/doi/10.1145/2465351.2465373) provides another way to unify abstractions. It makes the software-level abstraction radix trees, similar to hardware page tables. And make the page tables cache the radix tree nodes. This design effective in terms of scalability while not suitable for Asterinas because the software-level abstraction cannot be implemented in safe code. And its Linux-compatibility is limited.

[^1]: [CortenMM: Efficient Memory Management with Strong Correctness Guarantees](https://doi.org/10.1145/3731569.3764836)

[^2]: [Scalable Address Spaces using Concurrent Interval Skiplist](https://dl.acm.org/doi/10.1145/3731569.3764807)

[^3]: [Concurrent page-fault handling with per-VMA locks](https://lore.kernel.org/all/20220901173516.702122-1-surenb@google.com/)

[^4]: [LATR: Lazy Translation Coherence](https://dl.acm.org/doi/10.1145/3296957.3173198)