# Memory Management

<!--
Put system calls such as 
brk, mmap, munmap, mprotect, mremap, msync, mincore, madvise, 
shmget, shmat, shmctl, mlock, munlock, mbind, and set_mempolicy
under this part.
-->

## Memory Mappings

### `mmap` and `munmap`

Supported functionality in SCML:

```c
prot = PROT_NONE |
    PROT_EXEC |
    PROT_READ |
    PROT_WRITE;
opt_flags =
    MAP_ANONYMOUS |
    MAP_FIXED |
    MAP_FIXED_NOREPLACE |
    MAP_GROWSDOWN |
    MAP_HUGETLB |
    MAP_LOCKED |
    MAP_NONBLOCK |
    MAP_NORESERVE |
    MAP_POPULATE |
    MAP_SYNC;

// Create a private memory mapping
mmap(
    addr, length,
    prot = <prot>, 
    flags = MAP_PRIVATE | <opt_flags>
    fd, offset
);
    
// Create a shared memory mapping
mmap(
    addr, length,
    prot = <prot>, 
    flags = MAP_SHARED | MAP_SHARED_VALIDATE | <opt_flags>
    fd, offset
);

// Unmap a memory mapping
munmap(addr, length);
```

Silently-ignored flags:
* `MAP_HUGETLB`
* `MAP_GROWSDOWN`
* `MAP_LOCKED`
* `MAP_NONBLOCK`
* `MAP_NORESERVE`
* `MAP_POPULATE`
* `MAP_SYNC`

Partially supported flags:
* `MAP_FIXED_NOREPLACE` is treated as `MAP_FIXED`

Unsupported flags:
* `MAP_32BIT`
* `MAP_HUGE_1GB`
* `MAP_HUGE_2MB`
* `MAP_UNINITIALIZED`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/mmap.2.html).

### `msync`

Supported functionality in SCML:

```c
// Flush memory region to disk asynchronously
msync(
    addr, length,
    flags = MS_ASYNC | MS_INVALIDATE
);

// Flush memory region to disk synchronously
msync(
    addr, length,
    flags = MS_SYNC | MS_INVALIDATE
);
```

Silently-ignored flags:
* `MS_INVALIDATE` is ignored because all processes use the same page cache

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/msync.2.html).

### `mremap`

Supported functionality in SCML:

```c
// Resize an existing memory mapping. Relocation is allowed if given `MREMAP_MAYMOVE`.
mremap(
    old_address,
    old_size,
    new_size,
    flags = MREMAP_MAYMOVE
);

// Resize an existing memory mapping and force relocation to a specified location.
mremap(
    old_address,
    old_size,
    new_size,
    flags = MREMAP_MAYMOVE | MREMAP_FIXED,
    new_address
);
```

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/mremap.2.html).

### `mprotect`

Supported functionality in SCML:

```c
// Set memory access permissions
mprotect(
    addr,
    len,
    prot = <prot>
);
```

Silently-ignored protection flags:
* `PROT_SEM`
* `PROT_SAO`
* `PROT_GROWSUP`
* `PROT_GROWSDOWN`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/mprotect.2.html).

### `madvise`

Supported functionality in SCML:

```c
// Apply the default memory access pattern with no special optimizations
madvise(addr, length, advice = MADV_NORMAL);

// Indicate sequential access to enable aggressive read-ahead and immediate page release
madvise(addr, length, advice = MADV_SEQUENTIAL);

// Prefetch pages for near-future access to reduce latency
madvise(addr, length, advice = MADV_WILLNEED);
```

Silently-ignored advice:
* `MADV_DONTNEED`

Unsupported advice:
* `MADV_RANDOM`
* `MADV_REMOVE`
* `MADV_DONTFORK`
* `MADV_DOFORK`
* `MADV_HWPOISON`
* `MADV_MERGEABLE`
* `MADV_UNMERGEABLE`
* `MADV_SOFT_OFFLINE`
* `MADV_HUGEPAGE`
* `MADV_NOHUGEPAGE`
* `MADV_DONTDUMP`
* `MADV_DODUMP`
* `MADV_FREE`
* `MADV_WIPEONFORK`
* `MADV_KEEPONFORK`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/madvise.2.html).
