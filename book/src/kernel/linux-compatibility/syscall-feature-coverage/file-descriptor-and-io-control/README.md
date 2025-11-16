# File Descriptor and I/O Control

<!--
Put system calls such as
dup, dup2, dup3, fcntl, ioctl, pipe, pipe2, splice, tee, vmsplice, sendfile,
eventfd, eventfd2, inotify_init, inotify_init1, inotify_add_watch, and inotify_rm_watch
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
* `F_ADD_SEALS` and `F_GET_SEALS`
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
