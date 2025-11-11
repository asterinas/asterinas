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
ignore_flags = O_RDONLY | O_WRONLY | O_RDWR | O_CREAT | O_EXCL | O_NOCTTY | O_TRUNC;
can_change_flags = O_APPEND | O_ASYNC | O_DIRECT | O_NOATIME | O_NONBLOCK;

// Duplicate a file descriptor
fcntl(fd, cmd = F_DUPFD | F_DUPFD_CLOEXEC, arg);

// Retrieve file descriptor flags (F_GETFD), file status flags (F_GETFL)
// or SIGIO/SIGURG owner process (F_GETOWN)
fcntl(fd, cmd = F_GETFD | F_GETFL | F_GETOWN);

// Set file descriptor flags
fcntl(fd, cmd = F_SETFD, arg = FD_CLOEXEC);

// Set file status flags
fcntl(fd, cmd = F_SETFL, arg = <ignore_flags> | <can_change_flags>);

// Manage record locks: test (F_GETLK), non-blocking set (F_SETLK), blocking set (F_SETLKW)
fcntl(fd, cmd = F_GETLK | F_SETLK | F_SETLKW, arg);

// Assign SIGIO/SIGURG owner process
fcntl(fd, cmd = F_SETOWN, arg);
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
// Create pipe
pipe(pipefd);

// Create pipe with enhanced behavior control
pipe2(pipefd, flags = O_CLOEXEC);
```

Silently-ignored flags:
* `O_DIRECT`
* `O_NONBLOCK`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/pipe.2.html).

### `eventfd` and `eventfd2`

Supported functionality in SCML:

```c
// Create event notification descriptor
eventfd(initval);

// Create event notification descriptor with enhanced behavior control
eventfd2(initval, flags = EFD_CLOEXEC);
```

Silently-ignored flags:
* `EFD_NONBLOCK`
* `EFD_SEMAPHORE`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/eventfd.2.html).
