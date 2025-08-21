# cargo osdk debug

## Overview

`cargo osdk debug` is used to debug a remote target via GDB. You need to start
a running server to debug with. This is accomplished by the `run` subcommand
with `--gdb-server`. Then you can use the following command to attach to the
server and do debugging.

```bash
cargo osdk debug [OPTIONS]
```

Note that when KVM is enabled, hardware-assisted break points (`hbreak`) are
needed instead of the normal break points (`break`/`b`) in GDB.

## Options

`--remote <REMOTE>`:
Specify the address of the remote target [default: .osdk-gdb-socket].
The address can be either a path for the UNIX domain socket
or a TCP port on an IP address.

## Examples

To debug a remote target started with
[QEMU GDB stub](https://www.qemu.org/docs/master/system/gdb.html) or the `run`
subcommand, use the following commands.

Connect to an unix socket, e.g., `./debug`:

```bash
cargo osdk debug --remote ./debug
```

Connect to a TCP port (`[IP]:PORT`), e.g., `localhost:1234`:

```bash
cargo osdk debug --remote localhost:1234
```
