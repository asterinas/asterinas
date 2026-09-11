# EXT4 PR 01 Final Fix Report

## Required fixes

- Extent traversal now validates the complete supported tree before lookup and before mutation. It rejects overlapping data ranges, duplicate external leaf blocks, and external-node/data aliasing with `EUCLEAN`.
- Truncation and obsolete external-node release errors are returned rather than logged and suppressed. Sector accounting updates use the blocks actually released before returning the release error.
- The EXT4 memory-disk mock reports partial BIO spans with ceiling block division.
- The unknown-compatible-feature allocation test now allocates one block and verifies the persisted free-block counter decreases while the unknown compatible bit remains set.
- `FeatureCompatSet::from_bits_retain` now documents why compatible unknown bits are retained and why unsupported behavior is still mount-gated.

## Disputed findings

- Low `i_blocks` panic: disproved for a fully validated tree. Truncation prevalidates `sector_count >= (planned data releases + anticipated external releases) * SECTORS_PER_BLOCK`; `anticipated.freed` is exactly the validated root external-entry count, which is the rebuilt delta. Allocation prevalidates the corresponding addition. The post-rebuild `expect`s were nevertheless removed, so a violated invariant returns the ordinary accounting error instead of panicking.
- Forgotten page-cache writeback error: confirmed. A failed completion cleared `is_writing_back`, left the page clean, and later flushes selected only dirty or in-flight pages. Cache-page metadata now latches completion failures; `flush_dirty_pages` selects and reports such pages until a successful writeback clears the latch. The new deferred-write BIO test covers a failed completion followed by a later flush.

## Tests and verification

- `cargo fmt` and `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo check -p aster-core --no-default-features`: blocked before compilation because Cargo could not fetch the pinned `smoltcp` git revision. The sandbox attempt failed with network access; the approved retry reached Cargo's cache but failed permission to create pack files under `/home/zn/.cargo/git`.
- Kernel `ktest` execution was not feasible for the same unresolved dependency/cache blocker.

## Commit

Temporary fix commit: recorded after this report is staged.

## Concerns

- Full kernel-mode test execution remains required once the pinned Cargo git dependency is available to the build user.
