# File Descriptor and I/O Control

<!--
Put system calls such as
dup, dup2, dup3, fcntl, ioctl, pipe, pipe2, splice, tee, vmsplice, sendfile,
eventfd, eventfd2, memfd_create and fadvise64
under this category.
-->

### `fcntl`

Supported functionality in SCML:

```c
{{#include fcntl.scml}}
```

Unsupported commands:
* `F_NOTIFY`
* `F_OFD_SETLK`, `F_OFD_SETLKW` and `F_OFD_GETLK`
* `F_GETOWN_EX` and `F_SETOWN_EX`
* `F_GETSIG` and `F_SETSIG`
* `F_SETLEASE` and `F_GETLEASE`
* `F_SETPIPE_SZ` and `F_GETPIPE_SZ`
* `F_GET_RW_HINT` and `F_SET_RW_HINT`
* `F_GET_FILE_RW_HINT` and `F_SET_FILE_RW_HINT`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/fcntl.2.html).

### `pipe` and `pipe2`

Supported functionality in SCML:

```c
{{#include pipe_and_pipe2.scml}}
```

Silently-ignored flags:
* `O_DIRECT`
* `O_NONBLOCK`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/pipe.2.html).

### `eventfd` and `eventfd2`

Supported functionality in SCML:

```c
{{#include eventfd_and_eventfd2.scml}}
```

Silently-ignored flags:
* `EFD_NONBLOCK`
* `EFD_SEMAPHORE`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/eventfd.2.html).

### `memfd_create`

Supported functionality in SCML:

```c
{{#include memfd_create.scml}}
```

Silently-ignored flags:
* `MFD_HUGETLB`

Unsupported flags:
* `MFD_HUGE_64KB`
* `MFD_HUGE_512KB`
* `MFD_HUGE_1MB`
* `MFD_HUGE_2MB`
* `MFD_HUGE_8MB`
* `MFD_HUGE_16MB`
* `MFD_HUGE_32MB`
* `MFD_HUGE_256MB`
* `MFD_HUGE_512MB`
* `MFD_HUGE_1GB`
* `MFD_HUGE_2GB`
* `MFD_HUGE_16GB`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/memfd_create.2.html).

### `fadvise64`

Supported functionality in SCML:

```c
{{#include fadvise64.scml}}
```

Silently-ignored flags:
* `POSIX_FADV_NORMAL`
* `POSIX_FADV_RANDOM`
* `POSIX_FADV_SEQUENTIAL`
* `POSIX_FADV_WILLNEED`
* `POSIX_FADV_DONTNEED`
* `POSIX_FADV_NOREUSE`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/posix_fadvise.2.html).

### `epoll_ctl`

Supported functionality in SCML:

```c
{{#include epoll_ctl.scml}}
```

Unsupported flags in events:
* `EPOLLEXCLUSIVE`
* `EPOLLWAKEUP`

A seccomp listener reports `EPOLLOUT` and `EPOLLWRNORM` when a request
has been received and is awaiting a reply.

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/epoll_ctl.2.html).

### `poll` and `ppoll`

Supported functionality in SCML:

```c
{{#include poll_and_ppoll.scml}}
```

Unsupported events:
* `POLLRDBAND`
* `POLLWRBAND`

A seccomp listener reports `POLLOUT` and `POLLWRNORM` when a request
has been received and is awaiting a reply.

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/poll.2.html).

### `ioctl`

Supported functionality in SCML:

```c
{{#include ioctl.scml}}
```

#### Seccomp listener commands {#seccomp-listener-ioctls}

* `SECCOMP_IOCTL_NOTIF_RECV` receives a request into a zero-filled
  `struct seccomp_notif` buffer.
* `SECCOMP_IOCTL_NOTIF_SEND` replies with a `struct seccomp_notif_resp`.
  When `SECCOMP_USER_NOTIF_FLAG_CONTINUE` is set, both `val` and `error`
  must be zero.
* `SECCOMP_IOCTL_NOTIF_ID_VALID` checks whether a request is awaiting a reply.
  Both the current `_IOW` encoding and the legacy `_IOR` encoding read
  the request ID from a pointer to a 64-bit unsigned integer.
* `SECCOMP_IOCTL_NOTIF_ADDFD` takes a `struct seccomp_notif_addfd`.
  It installs the supervisor's `srcfd` in the notifying thread's file table
  and returns the new descriptor number. The request must be awaiting a reply,
  and the descriptor must be below the target's `RLIMIT_NOFILE`.
  * `SECCOMP_ADDFD_FLAG_SETFD` selects the exact descriptor in `newfd`
    and replaces any file installed there. Without this flag, set `newfd` to zero
    to let the kernel choose the lowest available descriptor.
  * `newfd_flags` accepts zero or `O_CLOEXEC`.
  * `SECCOMP_ADDFD_FLAG_SEND` installs the descriptor and replies atomically.
    The notified syscall returns the new descriptor number on success.
    A pending fd injection for the same request causes `EBUSY`.

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/ioctl.2.html).

### `ioprio_set` and `ioprio_get`

Supported functionality in SCML:

```c
{{#include ioprio_get_and_set.scml}}
```

Unsupported selectors:
* `IOPRIO_WHO_PGRP`
* `IOPRIO_WHO_USER`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/ioprio_set.2.html).
