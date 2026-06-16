# Kernel Parameters

This section documents kernel command-line parameters supported by Asterinas.

## Inherited from Linux

### `init`

Run the specified binary as `init`.

Example:
```text
init=/bin/busybox
```

Notes:
- The value is the path to the executable.
- If omitted, Asterinas will try to execute from the following paths in order:
  `/sbin/init`, `/etc/init`, `/bin/init`, `/bin/sh`.

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
