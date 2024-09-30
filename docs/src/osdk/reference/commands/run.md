# cargo osdk run

## Overview

`cargo osdk run` is used to run the kernel with QEMU.
The usage is as follows:

```bash
cargo osdk run [OPTIONS]
```

## Options

Most options are the same as those of `cargo osdk build`.
Refer to the [documentation](build.md) of `cargo osdk build`
for more details.

Options related with debugging:

- `--gdb-server`: Enable QEMU GDB server for debugging.
- `--gdb-wait-client`: Let the QEMU GDB server wait for the client connection before execution.
- `--gdb-vsc`: Generate a '.vscode/launch.json' for debugging kernel with Visual Studio Code
(only works when QEMU GDB server is enabled, i.e., `--gdb-server`).
Requires [CodeLLDB](https://marketplace.visualstudio.com/items?itemName=vadimcn.vscode-lldb).
- `--gdb-server-addr <ADDR>`: The network address on which the GDB server listens,
it can be either a path for the UNIX domain socket or a TCP port on an IP address.
[default: `.osdk-gdb-socket`(a local UNIX socket)]

See [Debug Command](debug.md) to interact with the GDB server in terminal.

## Examples

Launch a debug server via QEMU with an unix socket stub, e.g. `.debug`:

```bash
cargo osdk run --gdb-server --gdb-server-addr .debug
```

Launch a debug server via QEMU with a TCP stub, e.g., `localhost:1234`:

```bash
cargo osdk run --gdb-server --gdb-server-addr :1234
```

Launch a debug server via QEMU and use VSCode to interact:

```bash
cargo osdk run --gdb-server --gdb-vsc --gdb-server-addr :1234
```
