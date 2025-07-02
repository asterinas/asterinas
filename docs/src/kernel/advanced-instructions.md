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

### General Test

The following command builds and runs the test binaries in `test/src/apps` directory on Asterinas.

```bash
make run AUTO_TEST=test
```

### Syscall Test

The following command builds and runs the syscall test binaries on Asterinas.

```bash
make run AUTO_TEST=syscall
```

To run system call tests interactively, start an instance of Asterinas with the system call tests built and installed.

```bash
make run BUILD_SYSCALL_TEST=1
```

Then, in the interactive shell, run the following script to start the syscall tests.

```bash
/opt/syscall_test/run_syscall_test.sh
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
