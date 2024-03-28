# cargo osdk debug

## Overview

`cargo osdk debug` is used to debug a remote target via GDB.
The usage is as follows:

```bash
cargo osdk debug [OPTIONS]
```

## Options

`--remote <REMOTE>`:
Specify the address of the remote target [default: .aster-gdb-socket].
The address can be either a path for the UNIX domain socket
or a TCP port on an IP address.

## Examples

- To debug a remote target via a
[QEMU GDB stub](https://www.qemu.org/docs/master/system/gdb.html),
    - connect to an unix socket, e.g., `./debug`;
    ```bash
    cargo osdk debug --remote ./debug
    ```
    - connect to a TCP port (`[IP]:PORT`), e.g., `localhost:1234`.
    ```bash
    cargo osdk debug --remote localhost:1234
    ```
