# cargo osdk test

`cargo osdk test` is used to
execute kernel mode unit test by starting QEMU.
The usage is as follows:

```bash
cargo osdk test [TESTNAME] [OPTIONS] 
```

## Arguments 

`TESTNAME`:
Only run tests containing this string in their names

## Options

The options are the same as those of `cargo osdk build`.
Refer to the [documentation](build.md) of `cargo osdk build`
for more details.

## Examples
- Execute tests that include *foo* in their names 
using QEMU with 3GB of memory

```bash
cargo osdk test foo --qemu-args="-m 3G"
```
