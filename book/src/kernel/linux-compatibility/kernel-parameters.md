# Kernel Parameters

This section documents kernel command-line parameters supported by Asterinas.

## Inherited from Linux

### `rdinit`

Run the specified initramfs binary as the first userspace process.

Example:
```text
rdinit=/bin/busybox
```

Notes:
- The value is the path to the executable in the initramfs root.
- If omitted, Asterinas will try to execute `/init` from the initramfs root.

### `root`

Mount the specified block device as the real root filesystem when no initramfs
init is selected to run.

Example:
```text
root=/dev/vda2
```

Notes:
- The value currently must name a registered block device under `/dev`, such as
  `/dev/vda2`.
- By default, the initramfs init is `/init`; `rdinit` overrides it with another
  path.
- If the selected initramfs init is present, that program is responsible for
  switching to the real root filesystem.

### `rootfstype`

Select the filesystem type used for the real root filesystem.

Example:
```text
root=/dev/vda2 rootfstype=ext2
```

Valid values:
- `ext2`

### `init`

Run the specified executable as the first userspace process from the real root
filesystem.

Example:
```text
root=/dev/vda2 rootfstype=ext2 init=/nix/var/nix/profiles/system/init
```

Notes:
- `init` is used only after Asterinas mounts the real root filesystem via
  `root=`.
- If omitted, Asterinas tries `/sbin/init`, `/etc/init`, `/bin/init`, and
  `/bin/sh`, in that order.

### `console`

Select console devices for kernel messages.
This parameter may be specified multiple times.
Kernel messages are delivered to each listed console.

Valid values:
- `tty0`
- `ttyS0`
- `hvc0`

Examples:
```text
console=ttyS0
console=ttyS0 console=hvc0
```

## Asterinas-specific

### `ostd.log_level`

Set the verbosity level for Asterinas's logs.

Valid values (from most to least severe):
- `off`
- `emerg`
- `alert`
- `crit`
- `error`
- `warn` (alias: `warning`)
- `notice`
- `info`
- `debug`

Example:
```text
ostd.log_level=error
```

### `i8042.exist`

Override ACPI's indication of whether a PS/2 (i8042) controller exists.

Valid values:
- `1`, `on`, `yes`, `true` or no value — treat the i8042 controller as present (force probing)
- `0`, `off`, `no`, `false` - treat the i8042 controller as absent (skip probing)

Examples:
```text
i8042.exist
i8042.exist=1
i8042.exist=0
```
