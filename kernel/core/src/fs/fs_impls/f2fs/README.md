# F2FS Implementation Notes

This document describes the F2FS (Flash-Friendly File System) driver in
`kernel/core/src/fs/fs_impls/f2fs/`. It covers the design, the supported
feature set, how to use the driver, and the caveats that matter when testing
or extending it.

## 1. Design

### 1.1 On-disk layout

The driver understands the layout produced by `mkfs.f2fs` for 4 KiB blocks:

| Area | Meaning |
|------|---------|
| Superblock | Located at byte offset 1024 of block 0; describes every area |
| Checkpoint | Two checkpoint packs, each followed by summary blocks |
| SIT | Segment Information Table: per-segment valid-block bitmap |
| NAT | Node Address Table: maps node ids to node block addresses |
| SSA | Segment Summary Area: one summary block per main-area segment |
| Main area | File data blocks and node blocks (inode, direct, indirect) |

The checkpoint carries a version number, allocation counters, the pointers to
the current segments (`cur_node_segno` / `cur_data_segno`), and two bitmaps
that select the active copy of each NAT and SIT block. Both copies alternate
across the area, and the bitmaps are MSB-first. Directory entry bitmaps, in
contrast, are LSB-first.

### 1.2 Read path

1. Mount reads the superblock, validates the geometry (4 KiB blocks, 512-block
   segments, area boundaries, root inode inside the NAT capacity), and selects
   the checkpoint with the higher version.
2. The NAT journal stored in the selected checkpoint's node summary blocks is
   loaded; it overrides the NAT area for recently updated entries.
3. `Inode::load()` resolves the inode number through NAT, reads the node block,
   checks the node footer, and caches one `Inode` object per inode number.
4. File block indexes are resolved through the address tree:
   `i_addr` → two direct nodes → two indirect nodes → double-indirect node.
   Inline data and inline dentries are read from `i_addr` when the inode has
   the corresponding flags.
5. Directories are scanned slot by slot, advancing by the name length, so
   continuation slots of multi-slot names are skipped. Both block dentries and
   inline dentries are supported.
6. Symbolic links are read through the normal data path; small targets are
   inline, larger targets use data blocks.

All multi-byte values are read explicitly in little-endian order; the code does
not reinterpret byte buffers as structs.

### 1.3 Write path

The driver uses **in-place allocation** rather than F2FS's log-structured
relocation:

1. Free blocks are taken from the SIT, preferring segments that already carry
   the same kind of block (data or node). Segments named as current segments by
   the checkpoint are never allocated from, because their summary state lives in
   the checkpoint pack.
2. Every allocation or free updates the active SIT copy, writes a segment
   summary entry to the SSA, and updates NAT entries for newly allocated node
   ids. Changes are written through to the block device immediately.
3. Data writes allocate blocks on demand, support sparse holes, and extend the
   inode size. Truncation frees blocks and node blocks.
4. Inline data is materialized into a data block on the first write, and inline
   directories are converted to block-backed directories on the first mutation.
5. Directory entries are inserted into free slots and new dentry blocks are
   appended when a directory grows. Removal clears the slot bits.
6. `sync()` recomputes `valid_block_count`, `valid_node_count`,
   `free_segment_count`, `valid_inode_count` and `next_free_nid` and rewrites
   them, together with the checkpoint CRC32, into the selected checkpoint block
   in place. No new checkpoint pack is produced.
7. File and node blocks are restricted to the main area: addresses that point
   into the superblock, checkpoint, SIT, NAT or SSA areas are rejected with
   `EIO` instead of being read or written.

### 1.4 Locking

- `F2fs::alloc_state` is a filesystem-wide mutex for block/NAT/checkpoint
  allocation state.
- Each inode guards its mutable state with an `RwMutex`.
- Write paths take the allocation lock first and then inode locks; two-parent
  operations such as rename take the parent locks in inode-number order.
- Inode loads are cached atomically, and a freed inode is evicted from the
  cache so that a reused inode number cannot alias the old object.
- Child inodes are validated before a parent write lock is taken, avoiding
  lock-order inversion and self-deadlocks on corrupted directory entries.

## 2. Implemented Features

| Area | Supported |
|------|-----------|
| Mount | `mount -t f2fs <device> <mountpoint>`, read-write by default, `-o ro` supported |
| Read | Directories, regular files (inline and block-mapped), sparse files, symlinks, metadata, `readdir`, `statfs` |
| Create | Regular files, directories (including arbitrarily deep trees), symbolic links |
| Write | Sequential writes, appends, overwrite in place, sparse extension, `ftruncate` |
| Remove | `unlink` of files, `rmdir` of empty directories, link-count handling |
| Rename | Files and directories, within one directory or across directories; replacement of files and empty directories; `RENAME_NOREPLACE` via the VFS; `..` and parent inode updates |
| Metadata | `chmod`, `chown`, `utimensat` (mode/uid/gid/atime/mtime/ctime persisted) |
| Allocation | SIT-based in-place allocation, NAT updates, SSA summary updates, checkpoint counters and CRC on `sync` |
| Limits | 4 KiB blocks, 512-block segments, up to a 1 GiB directory scan, file size bounded by the address tree |

The regression suite under
`test/initramfs/src/regression/fs/f2fs/` exercises 273 assertions across these
areas, including a remount-persistence test.

## 3. Unimplemented Features and Limitations

Not implemented:

- Hard links (`link`), device nodes (`mknod`), fifos/sockets creation.
- Extended attributes (`getxattr`/`setxattr`/`listxattr`).
- `mmap` and page-cache integration; file I/O goes directly to the block
  device.
- `RENAME_EXCHANGE`.
- Garbage collection, TRIM/discard, log-structured relocation, and writing a
  new checkpoint pack (checkpoint counters/CRC are updated in place).
- Encryption, compression, fs-verity, casefold, zoned block devices, atomic
  writes, and multi-device volumes. Mounts with these feature bits are rejected
  with `EOPNOTSUPP`.

Limitations that follow from the current design:

- **No crash consistency.** Writes are write-through, but a crash in the middle
  of a multi-block metadata update can leave the volume inconsistent. The
  in-place checkpoint update is not atomic.
- **No orphan handling.** Unlinking or renaming over a file that is still open
  frees its blocks immediately; the open descriptor keeps stale metadata.
- **Linear directory lookup/iteration.** Directories are scanned linearly and
  NAT blocks are re-read on each lookup; there is no dentry/NAT cache.
- **Directory size cap.** Directories larger than 1 GiB are rejected during
  inode loading to bound scanning cost.
- **Mount options are ignored**, including the F2FS-specific options that Linux
  accepts; only the read-only mount flag is honored.
- **Symlink targets are treated as UTF-8**, because the VFS represents link
  targets as `String`.
- **`O_DIRECT` is accepted but not truly direct**; reads and writes use the
  same buffered-by-the-driver path.

## 4. How to Use

All commands below run inside the project's development container (see
`AGENTS.md`).

### 4.1 Boot with automatic mounting

On x86_64, the build creates a 128 MiB F2FS test image and QEMU attaches it as
the last virtio disk. The initramfs init process probes the virtio disks for
the F2FS magic number and mounts the first match at `/f2fs`:

```bash
make run_kernel
# inside the guest:
mount | grep f2fs
# /dev/vdd on /f2fs type f2fs (rw,relatime)
```

This works without enabling regression tests. On non-x86_64 configurations the
disk is not attached; mount a volume manually as shown below.

### 4.2 Manual mounting

```sh
mkdir -p /f2fs
mount -t f2fs /dev/vdd /f2fs          # read-write
mount -t f2fs -o ro /dev/vdd /f2fs    # read-only
umount /f2fs
```

`/proc/mounts` reports the filesystem type and `statfs().f_type` is the F2FS
magic number `0xF2F52010`.

### 4.3 Typical operations

```sh
echo hello > /f2fs/hello.txt
cat /f2fs/hello.txt
echo more >> /f2fs/hello.txt            # append / inline-data materialization
mkdir -p /f2fs/a/b/c
printf data > /f2fs/a/b/c/file
mv /f2fs/hello.txt /f2fs/a/b/renamed    # file rename across directories
mv /f2fs/a/b /f2fs/a/b2                 # directory rename
rm /f2fs/a/b2/c/file
rmdir /f2fs/a/b2/c
sync                                    # refresh checkpoint counters and CRC
umount /f2fs
```

After `sync`/`umount`, the same image also mounts with the Linux F2FS driver
(verified read-only with `mount -o loop,ro <image> <dir>`).

### 4.4 Running the tests

```bash
# Full regression suite (includes the F2FS tests and remount test)
make run_kernel AUTO_TEST=regression

# Lint checks (rustfmt, clippy, clang-format, nixfmt, typos)
make check
```

The F2FS tests live in:

- `test/initramfs/src/regression/fs/f2fs/f2fs_test.c`
- `test/initramfs/src/regression/fs/f2fs/f2fs_persist.c`

The test disk is regenerated from a compressed pristine image before every
x86_64 build, so tests start from a known state.

### 4.5 Regenerating the test image

`test/initramfs/src/f2fs/f2fs_test.img.gz` is generated by
`test/initramfs/src/f2fs/gen_test_image.sh`. The script needs:

- a privileged environment (to create and loop-mount the image),
- `mkfs.f2fs` (the `f2fs-tools` package),
- a host kernel with F2FS support,
- `python3` for the deterministic large-file patterns.

The file contents produced by the script are asserted by the C tests; keep
them in sync.

## 5. Test Results and Sample Output

The results below were produced on commit `f394fe6c1` (the F2FS driver and
integration commit) with the environment used in this repository: an x86_64
host, the `asterinas/dev` Docker image, QEMU with KVM, and the 128 MiB F2FS
test image generated from `f2fs_test.img.gz`.

### 5.1 Commands

```sh
make run_kernel AUTO_TEST=regression   # full regression, including F2FS
make check                             # rustfmt, clippy, clang-format, nixfmt, typos
make run_kernel                        # plain boot: F2FS auto-mounted at /f2fs
```

### 5.2 Aggregate results

- F2FS test functions: 19, running **273 assertions with 0 failures**.
- Remount-persistence assertions: 32 (included in the 273).
- Full regression: all 11 test directories passed and printed
  `All regression tests passed.`
- Lint: `make check` exited with status 0.
- The written image also mounts successfully with the Linux F2FS driver
  (read-only cross-check).

### 5.3 Per-function results

| Test function | Assertions | Failures |
|---------------|-----------:|---------:|
| `test_inline_data` | 6 | 0 |
| `test_large_file_content` | 3 | 0 |
| `test_sparse_file` | 10 | 0 |
| `test_directory_entries` | 23 | 0 |
| `test_symlinks` | 11 | 0 |
| `test_metadata` | 17 | 0 |
| `test_create_write_read` | 9 | 0 |
| `test_append_and_overwrite` | 12 | 0 |
| `test_truncate_and_holes` | 19 | 0 |
| `test_mkdir_write_delete` | 11 | 0 |
| `test_large_write` | 7 | 0 |
| `test_mount_type` | 6 | 0 |
| `test_persist_markers` | 16 | 0 |
| `test_multilevel_dirs` | 31 | 0 |
| `test_rename_file` | 23 | 0 |
| `test_rename_directory` | 29 | 0 |
| `test_statfs_reports_f2fs` | 8 | 0 |
| `test_persisted_files` (after remount) | 23 | 0 |
| `test_mount_is_f2fs` (after remount) | 9 | 0 |

### 5.4 Sample output

F2FS suite completion:

```text
test_multilevel_dirs summary: 31 tests passed, 0 tests failed
test_rename_file summary: 23 tests passed, 0 tests failed
test_rename_directory summary: 29 tests passed, 0 tests failed
test_persisted_files summary: 23 tests passed, 0 tests failed
test_mount_is_f2fs summary: 9 tests passed, 0 tests failed
All f2fs fs test passed.
```

Filesystem type check performed inside Asterinas:

```text
~ # mount | grep f2fs
/dev/vdd on /f2fs type f2fs (rw,relatime)
~ # grep " /f2fs f2fs " /proc/mounts
/dev/vdd /f2fs f2fs rw,relatime 0 0
~ # df -h /f2fs
Filesystem                Size      Used Available Use% Mounted on
/dev/vdd                 86.0M     31.0M     55.0M  36% /f2fs
```

`statfs("/f2fs").f_type` is asserted to equal `0xF2F52010` in
`test_mount_type` and `test_statfs_reports_f2fs`.

Plain boot without regression mode (automatic mount after removing the
regression-test gating):

```text
~ # mount | grep f2fs
/dev/vdd on /f2fs type f2fs (rw,relatime)
~ # grep " /f2fs f2fs " /proc/mounts
/dev/vdd /f2fs f2fs rw,relatime 0 0
~ # echo plain-final > /f2fs/plain_final.txt; cat /f2fs/plain_final.txt
plain-final
~ # rm /f2fs/plain_final.txt
```

Linux kernel cross-check of the image written by Asterinas:

```text
/dev/loop0 /mnt/check f2fs ro,...
fs_type=f2fs block_size=4096 total_blocks=32256 free_blocks=14081
--- renamed / multilevel entries ---
persist-a
keep
level4
ls: cannot access '/mnt/check/persist_a': No such file or directory
ls: cannot access '/mnt/check/persist_dir': No such file or directory
--- checksums ---
f2661b7889c9526f398e4101a52932ea  /mnt/check/crud_big
ef9d68a9a399031ae38c4c5a144879e0  /mnt/check/huge.bin
```

The two checksums match the patterns written by the regression test, which
confirms that file data, directory entries, renames and multi-level
directories are all readable by the reference implementation.

### 5.5 Preserved outputs

The complete outputs and the post-test image are kept in
`target/f2fs-artifacts/`:

| File | Contents |
|------|----------|
| `regression-full.log` | Full regression output (`All regression tests passed.`) |
| `f2fs-test-output.txt` | F2FS section with every assertion summary and mount output |
| `fs-suite.log` | Filesystem sub-suite output |
| `plain-boot-mount.log` | Plain boot with automatic F2FS mount and read/write/delete |
| `linux-mount-check.txt` | Linux F2FS mount, file contents and checksums |
| `code-review-f2fs.md` | Persona-based code review record |
| `f2fs-written.img` | Post-test 128 MiB image |

These paths are build artifacts and are not tracked by Git.

## 6. Additional Caveats

- **Always `sync` or `umount` before handing an image to another F2FS
  implementation.** The driver writes data and SIT/NAT/SSA metadata through,
  but the checkpoint counters and CRC are refreshed only on `sync`; another
  implementation may reject stale counters at mount time.
- **Do not use `fsck.f2fs` as a validation target.** Mounting the image with
  Linux F2FS (read-only) is the supported cross-check. Without garbage
  collection and checkpoint-pack alternation, an fsck may report issues that
  the driver intentionally does not repair.
- **The init script mounts only the first F2FS volume it finds.** Do not attach
  multiple F2FS disks if automatic mounting matters.
- **The test volume is 128 MiB and the regression tests modify it.** The build
  recreates it before each run; do not rely on its contents after a run unless
  you preserve the image yourself.
- **Allocation skips the checkpoint's current segments.** This keeps the volume
  mountable by Linux, but reduces the number of allocatable blocks. A full
  volume reports `ENOSPC`.
- **Checkpoint updates are in place.** There is a single active checkpoint
  block; power loss during `sync` can corrupt it, in which case the volume
  should be treated as damaged.
- **The driver accepts only 4 KiB blocks and 512-block segments.** Volumes with
  a different geometry are rejected at mount time.
- **Directory entry names are limited to 255 bytes** and are not
  case-insensitive; casefold volumes are rejected at mount.

## 7. Source Map

| File | Responsibility |
|------|----------------|
| `mod.rs` | Module documentation and VFS registration |
| `ondisk.rs` | On-disk structures, geometry validation, TEA dentry hash, CRC32 |
| `fs.rs` | Mount, checkpoint selection, NAT lookup/journal, block I/O, inode cache |
| `alloc.rs` | SIT allocator, NAT/SSA updates, checkpoint counter/CRC flush |
| `inode.rs` | Inode parsing, address tree, data I/O, directories, symlinks, create/remove/rename/truncate |
| `impl_for_vfs/` | `FileSystem`, `FileOps` and `Inode` adapters |
| `fs_type.rs` | `FsType` registration (`"f2fs"`) |
| `prelude.rs` | Shared imports |
