# File Systems & Mount Control

<!--
Put system calls such as
mount, umount2, pivot_root, statfs, fstatfs, truncate, ftruncate, fsync,
fdatasync, sync, syncfs, sync_file_range, open_tree, move_mount, fsopen,
fsconfig, fsmount, fspick, inotify_init, inotify_init1, inotify_add_watch,
inotify_rm_watch
under this category.
-->

## Mount and unmount file systems

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

## New mount API

### `fsconfig`

Supported functionality in SCML:

```c
{{#include fsconfig.scml}}
```

Unsupported commands:
* `FSCONFIG_SET_BINARY`
* `FSCONFIG_SET_PATH`
* `FSCONFIG_SET_PATH_EMPTY`
* `FSCONFIG_SET_FD`
* `FSCONFIG_CMD_CREATE_EXCL`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/fsconfig.2.html).

### `fsmount`

Supported functionality in SCML:

```c
{{#include fsmount.scml}}
```

Silently-ignored mount attributes:
* `MOUNT_ATTR_NOATIME`
* `MOUNT_ATTR_NODIRATIME`
* `MOUNT_ATTR_RELATIME`
* `MOUNT_ATTR_STRICTATIME`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/fsmount.2.html).

### `move_mount`

Supported functionality in SCML:

```c
{{#include move_mount.scml}}
```

Unsupported flags:
* `MOVE_MOUNT_F_SYMLINKS`
* `MOVE_MOUNT_F_AUTOMOUNTS`
* `MOVE_MOUNT_T_SYMLINKS`
* `MOVE_MOUNT_T_AUTOMOUNTS`
* `MOVE_MOUNT_T_EMPTY_PATH`
* `MOVE_MOUNT_SET_GROUP`
* `MOVE_MOUNT_BENEATH`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/move_mount.2.html).

## Event notifications

### `inotify_init` and `inotify_init1`

Supported functionality in SCML:

```c
{{#include inotify_init_and_init1.scml}}
```

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/inotify_init.2.html).

### `inotify_add_watch`

Supported functionality in SCML:

```c
{{#include inotify_add_watch.scml}}
```

Unsupported event flags:
* `IN_MOVED_FROM` and `IN_MOVED_TO` - Move events are not generated
* `IN_MOVE_SELF` - Self move events are not generated
* `IN_UNMOUNT` - Unmount events are not generated
* `IN_Q_OVERFLOW` - Queue overflow events are not generated (events are silently dropped when queue is full)
* `IN_ALL_EVENTS` - Only includes actually supported events

Unsupported control flags:
* `IN_EXCL_UNLINK` - Events on unlinked files are not excluded

For more information,
see [the man page](https://man7.org/linux/man-pages/man7/inotify.7.html).
