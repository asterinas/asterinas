# cargo osdk new

## Overview

The `cargo osdk new` command
is used to create a kernel project
or a new library project.
The usage is as follows:

```bash
cargo osdk new [OPTIONS] <name>
```

## Arguments

`<name>`: the name of the crate.

## Options

`--kernel`:
Use the kernel template.
If this option is not set,
the library template will be used by default.

`--library`:
Use the library template. This is the default option.

## Examples

- Create a new kernel named `myos`: 

```bash
cargo osdk new --kernel myos
```

- Create a new library named `mylib`:

```bash
cargo osdk new mylib
```
