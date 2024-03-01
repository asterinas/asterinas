# cargo osdk test

`cargo osdk test` is used to
execute kernel mode unit test by starting QEMU.
The usage is as follows:

```bash
cargo osdk test [OPTIONS] [TESTNAME]
```

## Arguments 

[TESTNAME]:
Only run tests containing this string in their names

## Options

The options are the same as those of `cargo osdk build`.
Refer to the [documentation](build.md) of `cargo osdk build`
for more details.

## Examples
- Execute tests containing foo in their names
with q35 as the QEMU machine type:

```bash
cargo osdk test foo --qemu.machine="q35"
```
