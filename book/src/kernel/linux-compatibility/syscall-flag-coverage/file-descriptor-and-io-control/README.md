# File Descriptor and I/O Control

<!--
Put system calls such as
dup, dup2, dup3, fcntl, ioctl, pipe, pipe2, splice, tee, vmsplice, sendfile,
eventfd, eventfd2 and memfd_create
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

### `epoll_ctl`

Supported functionality in SCML:

```c
{{#include epoll_ctl.scml}}
```

Unsupported flags in events:
* `EPOLLEXCLUSIVE`
* `EPOLLWAKEUP`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/epoll_ctl.2.html).

### `poll` and `ppoll`

Supported functionality in SCML:

```c
{{#include poll_and_ppoll.scml}}
```

Unsupported events:
* `POLLRDBAND`
* `POLLWRNORM`
* `POLLWRBAND`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/poll.2.html).

### `ioctl`

Supported functionality in SCML:

```c
{{#include ioctl.scml}}
```

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/ioctl.2.html).
