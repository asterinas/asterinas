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
    mountflags = MS_BIND | MS_REC | MS_MOVE,
    data
);
```

Silently-ignored mount flags:
* `MS_DIRSYNC`
* `MS_LAZYTIME`
* `MS_MANDLOCK`
* `MS_NOATIME`
* `MS_NODEV`
* `MS_NODIRATIME`
* `MS_NOEXEC`
* `MS_NOSUID`
* `MS_RDONLY`
* `MS_RELATIME`
* `MS_SILENT`
* `MS_STRICTATIME`
* `MS_SYNCHRONOUS`

Partially supported mount flags:
* `MS_REC` is only effective when used in conjunction with `MS_BIND`

Unsupported mount flags:
* `MS_REMOUNT`
* `MS_SHARED`
* `MS_PRIVATE`
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