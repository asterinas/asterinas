# File and Directory Operations

<!--
Put system calls such as
open, openat, creat, close, read, write, readv, writev, pread64, 
pwrite64, lseek, stat, fstat, lstat, statx, mkdir, rmdir, link, 
unlink, rename, symlink, readlink, chmod, fchmod, chown, fchown, 
utime, and utimensat
under this category.
-->

## `open` and `openat`

Supported functionality of `open` in SCML:

```c
access_mode =
    O_RDONLY |
    O_WRONLY |
    O_RDWR;
creation_flags =
    O_CLOEXEC |
    O_DIRECTORY |
    O_EXCL |
    O_NOCTTY |
    O_NOFOLLOW |
    O_TRUNC;
status_flags =
    O_APPEND |
    O_ASYNC |
    O_DIRECT |
    O_LARGEFILE |
    O_NOATIME |
    O_NONBLOCK |
    O_SYNC;

// Open an existing file
open(
    path,
    flags = <access_mode> | <creation_flags> | <status_flags>,
);

// Create a new file
open(
    path,
    flags = O_CREAT | <access_mode> | <creation_flags> | <status_flags>,
    mode
);

// Status flags that are meaningful with O_PATH
opath_valid_flags = O_CLOEXEC | O_DIRECTORY | O_NOFOLLOW;
// All other flags are ignored with O_PATH
opath_ignored_flags = O_CREAT | <creation_flags> | <status_flags>;
// Obtain a file descriptor to indicate a location in FS
open(
    path,
    flags = O_PATH | <opath_valid_flags> | <opath_ignored_flags>
);

// Create an unnamed file 
// open(path, flags = O_TMPFILE | <creation_flags> | <status_flags>) 
```

Silently-ignored flags:
* `O_NOCTTY`
* `O_DSYNC`
* `O_SYNC`
* `O_LARGEFILE`
* `O_NOATIME`
* `O_NOCTTY`

Partially-supported flags:
* `O_PATH`

Unsupported flags:
* `O_TMPFILE`

Supported and unsupported functionality of `openat` are the same as `open`.
The SCML rules are omitted for brevity.

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/openat.2.html).
