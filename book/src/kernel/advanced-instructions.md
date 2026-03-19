# Advanced Build and Test Instructions

## User-Mode Unit Tests

Asterinas consists of many crates,
some of which do not require a VM environment
and can be tested with the standard `cargo test`.
They are listed in the root `Makefile`
and can be tested together through the following Make command.

```bash
make test
```

To test an individual crate, enter the directory of the crate and invoke `cargo test`.

### Kernel-Mode Unit Tests

Many crates in Asterinas do require a VM environment to be tested.
The unit tests for these crates are empowered by OSDK.

```bash
make ktest
```

To test an individual crate in kernel mode, enter the directory of the crate and invoke `cargo osdk test`.

```bash
cd asterinas/ostd
cargo osdk test
```

## Integration Test

### Regression Test

The following command builds and runs the regression test in `test/initramfs/src/regression` on Asterinas.

```bash
make run_kernel AUTO_TEST=regression
```

### Conformance Test

The following command builds and runs the conformance test on Asterinas.

```bash
make run_kernel AUTO_TEST=conformance
```

To run conformance test interactively, start an instance of Asterinas with the conformance tests built and installed.

```bash
make run_kernel ENABLE_CONFORMANCE_TEST=true
```

Then, in the interactive shell, run the following script to start the conformance test.

```bash
/opt/run_conformance_test.sh
```

## Debug

### Using GDB to Debug

To debug Asterinas via [QEMU GDB support](https://qemu-project.gitlab.io/qemu/system/gdb.html),
you can compile Asterinas in the debug profile,
start an Asterinas instance and run the GDB interactive shell in another terminal.

Start a GDB-enabled VM of Asterinas with OSDK and wait for debugging connection:

```bash
make gdb_server
```

The server will listen at the default address specified in `Makefile`, i.e., a local TCP port `:1234`.
Change the address in `Makefile` for your convenience,
and check `cargo osdk run -h` for more details about the address.

Two options are provided to interact with the debug server.

- A GDB client: start a GDB client in another terminal.

    ```bash
    make gdb_client
    ```

- VS Code: [CodeLLDB](https://marketplace.visualstudio.com/items?itemName=vadimcn.vscode-lldb) extension is required.
After starting a debug server with OSDK from the shell with `make gdb_server`,
a temporary `launch.json` is generated under `.vscode`.
Your previous launch configs will be restored after the server is down.
Press `F5`(Run and Debug) to start a debug session via VS Code. 
Click `Continue`(or, press `F5`) at the first break to resume the paused server instance,
then it will continue until reaching your first breakpoint. 

Note that if debugging with KVM enabled, you must use hardware assisted breakpoints. See "hbreak" in
[the GDB manual](https://ftp.gnu.org/old-gnu/Manuals/gdb/html_node/gdb_28.html) for details.
