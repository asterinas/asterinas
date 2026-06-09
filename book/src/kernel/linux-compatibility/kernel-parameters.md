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
- If the specified path cannot be accessed,
  Asterinas ignores it and proceeds with root filesystem initialization.
  It does not fall back to `/init` in this case.

### `root`

Mount the specified block device as the root filesystem
when no initramfs init is selected to run.

Example:
```text
root=/dev/vda2
```

Notes:
- The value currently must name a registered block device under `/dev`,
  such as `/dev/vda2`.
- By default, the initramfs init is `/init`;
  `rdinit` overrides it with another path.
- If the selected initramfs init is present,
  that program is responsible for switching to the root filesystem.

### `rootfstype`

Select the filesystem type used for the root filesystem.

Example:
```text
root=/dev/vda2 rootfstype=ext2
```

Valid values:
- `ext2`

### `ro`

Mount the root filesystem read-only.
This is the default when Asterinas mounts the root filesystem via `root=`.

Example:
```text
root=/dev/vda2 ro
```

Notes:
- This parameter is ignored when an initramfs init is selected to run.
- If both `ro` and `rw` are specified, the last one takes precedence.

### `rw`

Mount the root filesystem read-write.

Example:
```text
root=/dev/vda2 rw
```

Notes:
- This parameter is ignored when an initramfs init is selected to run.
- `rw` overrides the default read-only root mount.
- If both `ro` and `rw` are specified, the last one takes precedence.

### `init`

Run the specified executable as the first userspace process from the root filesystem.

Example:
```text
root=/dev/vda2 rootfstype=ext2 init=/nix/var/nix/profiles/system/init
```

Notes:
- `init` is used only after Asterinas mounts the root filesystem via `root=`.
- If omitted,
  Asterinas tries `/sbin/init`, `/etc/init`, `/bin/init`, and `/bin/sh`, in that order.

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

### `virtio_mmio.device`

Register a VirtIO-MMIO device from the kernel command line.
This parameter may be specified multiple times.

Format:
```text
virtio_mmio.device=<size>@<base>:<irq>[:<id>]
```

Notes:
- `size` and `base` may be decimal or hexadecimal with a `0x` prefix.
- `size` may use `K`, `M`, `G`, or `T` suffixes.
- `irq` must be nonzero.
- The optional `id` field is accepted for Linux compatibility but ignored.

Examples:
```text
virtio_mmio.device=0x200@0x5950f000:10
virtio_mmio.device=1K@0x1001e000:74
```

## Asterinas-specific
### `earlycon`

Enable the early console to output logs during the early stages of system boot.
The name follows Linux's `earlycon` parameter.
Asterinas currently supports a simplified form.

Example:
```text
earlycon
```

Notes:
- If omitted, the early console stays disabled.
- Only the bare `earlycon` token is supported;
  complex Linux forms such as `earlycon=uart8250,io,0x3f8,115200` are not supported yet.

### `loglevel`

Control how verbose kernel log output is on the console.
Set either a numeric value (`0` to `8`) or a lowercase level name.

Each value acts as a cutoff: messages at that severity and all more urgent levels are printed.
For example, `loglevel=4` shows emergencies through errors, but not warnings or routine info.

This uses the same `0`–`8` scale as the Linux kernel `loglevel` parameter,
with string aliases for convenience.

| Value | Name(s)            | Messages shown              |
|------:|--------------------|-----------------------------|
| `0`   | `off`              | None                        |
| `1`   | `emerg`            | Emerg only                  |
| `2`   | `alert`            | Emerg through Alert         |
| `3`   | `crit`             | Emerg through Crit          |
| `4`   | `error`, `err`     | Emerg through Error         |
| `5`   | `warning`, `warn`  | Emerg through Warning       |
| `6`   | `notice`           | Emerg through Notice        |
| `7`   | `info`             | Emerg through Info          |
| `8`   | `debug`            | All levels (most verbose)   |

Example:
```text
loglevel=4
loglevel=error
```

Notes:
- Level names are case-sensitive; use lowercase names.
- If omitted, the default is `8` (`debug`). Invalid values are ignored.
- Use `warn` for normal operation, `info`/`debug` when troubleshooting,
  and `error` or lower for a quieter console.

## Asterinas-specific

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
