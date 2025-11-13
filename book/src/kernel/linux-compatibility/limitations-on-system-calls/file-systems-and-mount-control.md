# File Systems & Mount Control

<!--
Put system calls such as
mount, umount2, pivot_root, statfs, fstatfs, truncate, ftruncate, fsync, 
fdatasync, sync, syncfs, sync_file_range, open_tree, move_mount, fsopen,
fsconfig, fsmount, fspick, inotify_init, inotify_init1, inotify_add_watch,
inotify_rm_watch
under this category.
-->

## Mount and Unmount File Systems

### `mount`

Supported functionality in SCML:

```c
// Create a new mount
mount(
    source, target, filesystemtype,
    mountflags = 0,
    data
);

// Move the existing mount point
mount(
    source, target, filesystemtype,
    mountflags = MS_MOVE,
    data
);

// Create a bind mount
mount(
    source, target, filesystemtype,
    mountflags = MS_BIND | MS_REC,
    data
);
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
// Unmount a mounted file system
umount(target);

// Unmount a mounted file system with enhanced behavior control
umount2(target, flags = UMOUNT_NOFOLLOW);
```

Silently-ignored flags:
* `MNT_FORCE`
* `MNT_DETACH`
* `MNT_EXPIRE`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/umount.2.html).

## Event Notifications

### `inotify_init` and `inotify_init1`

Supported functionality in SCML:

```c
// Create an inotify instance
int fd = inotify_init();

// Create an inotify instance with enhanced behavior control
int fd = inotify_init1(flags = IN_CLOEXEC | IN_NONBLOCK);
```

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/inotify_init.2.html).

### `inotify_add_watch`

Supported functionality in SCML:

```c
inotify_events = IN_ACCESS | IN_MODIFY | IN_ATTRIB | IN_CLOSE_WRITE | 
                 IN_CLOSE_NOWRITE | IN_OPEN | IN_CREATE | IN_DELETE | 
                 IN_DELETE_SELF | IN_CLOSE;

inotify_controls = IN_ONLYDIR | IN_DONT_FOLLOW | IN_MASK_CREATE | 
                   IN_MASK_ADD | IN_ISDIR;

// Add a watch to an initialized inotify instance
int wd = inotify_add_watch(fd, pathname, <inotify_events> | <inotify_controls>);
```

Unsupported event flags:
* `IN_MOVED_FROM` and `IN_MOVED_TO` - Move events are not generated
* `IN_MOVE_SELF` - Self move events are not generated
* `IN_UNMOUNT` - Unmount events are not generated
* `IN_Q_OVERFLOW` - Queue overflow events are not generated (events are silently dropped when queue is full)
* `IN_ALL_EVENTS` - Only includes actually supported events

Unsupported control flags:
* `IN_EXCL_UNLINK` - Events on unlinked files are not excluded
* `IN_ONESHOT` - Watches are not automatically removed after the first event

For more information,
see [the man page](https://man7.org/linux/man-pages/man7/inotify.7.html).

### `inotify_rm_watch`

Supported functionality in SCML:

```c
// Remove a watch from an inotify instance
int result = inotify_rm_watch(fd, wd);
```

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/inotify_rm_watch.2.html).