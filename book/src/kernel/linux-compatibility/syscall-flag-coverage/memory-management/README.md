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
{{#include mmap_and_munmap.scml}}
```

Silently-ignored flags:
* `MAP_HUGETLB`
* `MAP_GROWSDOWN`
* `MAP_LOCKED`
* `MAP_NONBLOCK`
* `MAP_NORESERVE`
* `MAP_POPULATE`

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
{{#include msync.scml}}
```

Silently-ignored flags:
* `MS_INVALIDATE` is ignored because all processes use the same page cache

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/msync.2.html).

### `mremap`

Supported functionality in SCML:

```c
{{#include mremap.scml}}
```

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/mremap.2.html).

### `mprotect`

Supported functionality in SCML:

```c
{{#include mprotect.scml}}
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
{{#include madvise.scml}}
```

Silently-ignored advice:
* `MADV_NORMAL`
* `MADV_RANDOM`
* `MADV_SEQUENTIAL`
* `MADV_WILLNEED`
* `MADV_FREE`
* `MADV_MERGEABLE`
* `MADV_UNMERGEABLE`
* `MADV_HUGEPAGE`
* `MADV_NOHUGEPAGE`

Unsupported advice:
* `MADV_RANDOM`
* `MADV_REMOVE`
* `MADV_DONTFORK`
* `MADV_DOFORK`
* `MADV_HWPOISON`
* `MADV_UNMERGEABLE`
* `MADV_SOFT_OFFLINE`
* `MADV_DONTDUMP`
* `MADV_DODUMP`
* `MADV_FREE`
* `MADV_WIPEONFORK`
* `MADV_KEEPONFORK`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/madvise.2.html).
