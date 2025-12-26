# File and Directory Operations

<!--
Put system calls such as
open, openat, creat, close, read, write, readv, writev, pread64, 
pwrite64, lseek, stat, fstat, lstat, statx, mkdir, rmdir, link, 
unlink, rename, symlink, readlink, chmod, fchmod, chown, fchown, 
utime, and utimensat
under this category.
-->

### `open` and `openat`

Supported functionality of `open` in SCML:

```c
{{#include open_and_openat.scml}}
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

### `renameat2`

Supported functionality in SCML:

```c
{{#include renameat2.scml}}
```

Unsupported flags:
* `RENAME_EXCHANGE`
* `RENAME_NOREPLACE`
* `RENAME_WHITEOUT`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/rename.2.html).

### `lseek`

Supported functionality in SCML:

```c
{{#include lseek.scml}}
```

Unsupported flags:
* `SEEK_DATA`
* `SEEK_HOLE`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/lseek.2.html).

### `newfstatat`

Supported functionality in SCML:

```c
{{#include newfstatat.scml}}
```

Silently-ignored flags:
* `AT_NO_AUTOMOUNT`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/newfstatat.2.html).

### `preadv2` and `pwritev2`

Supported functionality in SCML:

```c
{{#include preadv2_and_pwritev2.scml}}
```

Silently-ignored flags:
* `RWF_DSYNC`
* `RWF_HIPRI`
* `RWF_SYNC`
* `RWF_NOWAIT`

Unsupported flags:
* `RWF_APPEND`
* `RWF_NOAPPEND`
* `RWF_ATOMIC`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/preadv2.2.html).

### `faccessat2`

Supported functionality in SCML:

```c
{{#include faccessat2.scml}}
```

Silently-ignored flags:
* `AT_EACCESS`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/faccessat2.2.html).

### `statx`

Supported functionality in SCML:

```c
{{#include statx.scml}}
```

Silently-ignored flags:
* `AT_NO_AUTOMOUNT`
* `AT_STATX_FORCE_SYNC`
* `AT_STATX_DONT_SYNC`

Silently-ignored masks:
* `STATX_DIOALIGN`
* `STATX_MNT_ID_UNIQUE`
* `STATX_SUBVOL`
* `STATX_WRITE_ATOMIC`
* `STATX_DIO_READ_ALIGN`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/statx.2.html).

### `fallocate`

Supported functionality in SCML:

```c
{{#include fallocate.scml}}
```

Unsupported modes:
* `FALLOC_FL_UNSHARE_RANGE`
* `FALLOC_FL_COLLAPSE_RANGE`
* `FALLOC_FL_ZERO_RANGE`
* `FALLOC_FL_INSERT_RANGE`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/fallocate.2.html).

### `utimensat`

Supported functionality in SCML:

```c
{{#include utimensat.scml}}
```

Unsupported flags:
* `AT_EMPTY_PATH`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/utimensat.2.html).
