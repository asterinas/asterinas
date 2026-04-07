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

## Asterinas-specific

### `ostd.log_level`

Set the verbosity level for Asterinas's logs.

Valid values:
- `off`
- `error`
- `warn`
- `info`
- `debug`
- `trace`

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
