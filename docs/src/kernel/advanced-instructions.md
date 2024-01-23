# Advanced Build and Test Instructions

## User-Mode Unit Tests

Asterinas consists of many crates, some of which do not require a VM environment and can be tested with the standard `cargo test`. They are listed in the root `Makefile` and can be tested together through the following Make command.

```bash
make test
```

To test an individual crate, enter the directory of the crate and invoke `cargo test`.

### Kernel-Mode Unit Tests

Many crates in Asterinas do require a VM environment to be tested. The unit tests for these crates are empowered by our [ktest](framework/libs/ktest) framework.

```bash
make run KTEST=1
```

It is also possible to specify a subset of tests to run.

```bash
make run KTEST=1 KTEST_WHITELIST=failing_assertion,aster_frame::test::expect_panic KTEST_CRATES=aster-frame
```

## Integration Test

### Regression Test

The following command builds and runs the test binaries in `regression/apps` directory on Asterinas.

```bash
make run AUTO_TEST=regression
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

To debug Asterinas using [QEMU GDB remote debugging](https://qemu-project.gitlab.io/qemu/system/gdb.html), one could compile Asterinas in the debug profile, start an Asterinas instance and run the GDB interactive shell in another terminal.

First, start a GDB-enabled VM of Asterinas and wait for debugging connection:

```bash
make run GDB_SERVER=1 ENABLE_KVM=0
```

Next, start a GDB client in another terminal.

```bash
make run GDB_CLIENT=1
```

Currently, the Asterinas runner's debugging interface is exposed via UNIX sockets. Thus there shouldn't be multiple debugging instances in the same container. To add debug symbols for the underlying infrastructures such as UEFI firmware or bootloader, please check the runner's source code for details.
