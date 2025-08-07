# Memory Management

<!--
Put system calls such as 
brk, mmap, munmap, mprotect, mremap, msync, mincore, madvise, 
shmget, shmat, shmctl, mlock, munlock, mbind, and set_mempolicy
under this part.
-->

## `mmap`

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
