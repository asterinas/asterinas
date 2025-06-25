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

Additionally, when running the kernel using QEMU, we can setup the QEMU as a
debug server using option `--gdb-server`. This option supports an additional
comma separated configuration list:

 - `addr=ADDR`: the network or unix socket address on which the GDB server listens
    (default: `.osdk-gdb-socket`, a local UNIX socket);
 - `wait-client`: let the GDB server wait for the GDB client before execution;
 - `vscode`: generate a '.vscode/launch.json' for debugging with Visual Studio Code
    (Requires [CodeLLDB](https://marketplace.visualstudio.com/items?itemName=vadimcn.vscode-lldb)).

Besides, to collect coverage data, we can use option `--coverage`. This option
enables the coverage feature and collect coverage data to `coverage.profraw` when exit.
It actually does several things:

 - It adds `-Cinstrument-coverage -Zno-profiler-runtime` to `RUSTFLAGS` so LLVM will
   generate coverage instrument. And `coverage` features will be enabled for `ostd`,
   then before `exit_qemu` actually quit QEMU, it will call `minicov` to collect
   the coverage data to guest's own memory, and print its address and size, so that
   OSDK can dump it out of guest.
 - Next, `--no-shutdown` will be enabled for QEMU, and OSDK will setup a monitor
   connection to QEMU to monitor its status. Once exit, it dumps the coverage data
   from guest's memory to `coverage.profraw`.

**Note.** The code coverage feature of OSDK requires a non-default OSTD feature called
`coverage`, which relies on dependencies that are not published on crates.io.
To utilize this feature, projects must specify OSTD as a dependency using a Git
repository or a local filesystem path in their `Cargo.toml`. For example:

```toml
[dependencies]
ostd = { git = "https://github.com/asterinas/asterinas", rev = "v0.11.0", features = ["coverage"] }
```

See [Debug Command](debug.md) to interact with the GDB server in terminal.

## Examples

Launch a debug server via QEMU with an unix socket stub, e.g. `.debug`:

```bash
cargo osdk run --gdb-server addr=.debug

```bash
cargo osdk run --gdb-server --gdb-server-addr .debug
```

Launch a debug server via QEMU with a TCP stub, e.g., `localhost:1234`:

```bash
cargo osdk run --gdb-server addr=:1234
```

Launch a debug server via QEMU and use VSCode to interact with:

```bash
cargo osdk run --gdb-server wait-client,vscode,addr=:1234
```

Launch a debug server via QEMU and use VSCode to interact with:

```bash
cargo osdk run --gdb-server --gdb-vsc --gdb-server-addr :1234
```
