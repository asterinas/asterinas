# File Systems & Mount Control

<!--
Put system calls such as
mount, umount2, pivot_root, statfs, fstatfs, truncate, ftruncate, fsync, 
fdatasync, sync, syncfs, sync_file_range, open_tree, move_mount, fsopen,
fsconfig, fsmount, and fspick
under this category.
-->

## Mount and Unmount File Systems

### `mount`

Supported functionality in SCML:

```c
{{#include mount.scml}}
```

Partially supported mount flags:
* `MS_REC` is only effective when used in conjunction with `MS_BIND`
* `MS_REMOUNT` can be used, but the set options have no actual effect.
* `MS_DIRSYNC` can be set but have no actual effect.
* `MS_LAZYTIME` can be set but have no actual effect.
* `MS_MANDLOCK` can be set but have no actual effect.
* `MS_NOATIME` can be set but have no actual effect.
* `MS_NODEV` can be set but have no actual effect.
* `MS_NODIRATIME` can be set but have no actual effect.
* `MS_NOEXEC` can be set but have no actual effect.
* `MS_NOSUID` can be set but have no actual effect.
* `MS_RDONLY` can be set but have no actual effect.
* `MS_RELATIME` can be set but have no actual effect.
* `MS_SILENT` can be set but have no actual effect.
* `MS_STRICTATIME` can be set but have no actual effect.
* `MS_SYNCHRONOUS` can be set but have no actual effect.

Unsupported mount flags:
* `MS_SHARED`
* `MS_SLAVE`
* `MS_UNBINDABLE`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/mount.2.html).

### `umount` and `umount2`

Supported functionality in SCML:

```c
{{#include umount_and_umount2.scml}}
```

Silently-ignored flags:
* `MNT_FORCE`
* `MNT_DETACH`
* `MNT_EXPIRE`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/umount.2.html).