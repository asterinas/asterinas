# Initramfs-Based Test Suites

This directory contains the test suites of Asterinas running in initramfs, including in-house syscall regression tests, third-party Linux conformance suites, benchmarks, configuration files, and Nix expressions to package them.

## Directory Structure

```
initramfs/
├── etc/               # Configuration files packaged into initramfs
├── nix/
│   ├── benchmark/     # Nix expressions for `benchmark`
│   ├── conformance/   # Nix expressions for `conformance`
│   ├── regression/    # Nix expressions for `regression`
│   └── initramfs.nix  # Nix expression for packaging initramfs
├── src/
│   ├── benchmark/     # Third-party benchmark suites to compare Asterinas against Linux
│   ├── conformance/   # Third-party test suites to verify Linux compatibility
│   │   ├── gvisor/    # Gvisor syscall test suite
│   │   ├── kselftest/ # Linux kernel in-tree selftests
│   │   └── ltp/       # Linux Test Project syscall test suite
│   ├── regression/    # In-house syscall and subsystem regression tests
│   └── boot_hello.sh  # Minimal boot smoke test script
├── Makefile
└── README.md
```

## Building and Packaging Tests

Most tests in this directory are compiled and packaged using [Nix](https://nixos.org/), a powerful package manager. This ensures consistency and reproducibility across environments.

> **Note**: If you are adding a new test to the `regression` directory, ensure that it supports multiple architectures. Some of the existing tests lack proper architecture-specific handling.

### Conformance Test Suite - gVisor Exception

While most tests rely on `Nix` for compilation, the `gvisor` conformance test suite currently cannot be built with `Nix`. Instead, the `gvisor` tests are compiled in the Docker image. For details, refer to `tools/docker/Dockerfile`.

### Linux Kernel Selftest (kselftest)

The `kselftest` suite builds a subset of Linux's in-tree selftests
(`tools/testing/selftests`) from the Linux version pinned in
`nix/conformance/kselftest.nix` (currently **v6.18**). Only the
subsystems listed in `baseKselftestTargets` are compiled, plus `x86`
on x86_64 hosts.

Invoke via:

```bash
make run_kernel AUTO_TEST=conformance CONFORMANCE_TEST_SUITE=kselftest
```

**Bumping the Linux version.** Edit `version` in
`nix/conformance/kselftest.nix`, temporarily set `hash = lib.fakeHash;`,
run the Nix build once, and copy the real hash reported by `fetchgit`
back into `hash`.

**Blocklist layout.** `src/conformance/kselftest/blocklists` is the
default blocklist; `blocklists.ext2` and `blocklists.exfat` hold
filesystem-specific extras. See the header of `blocklists` for the
entry format.

### Multi-Architecture Support

The test suite supports building for multiple architectures, including `x86_64` and `riscv64`. You can specify the desired architecture by running:

```bash
make kernel OSDK_TARGET_ARCH=x86_64
# or
make kernel OSDK_TARGET_ARCH=riscv64
```

The build artifacts (initramfs) can be found in the `test/initramfs/build` directory after the compilation.

## Supported Benchmarks

The following benchmarks are currently supported:

- fio
- hackbench
- iperf3
- lmbench
- memcached
- nginx
- redis
- schbench
- sqlite
- sysbench

### Architecture Compatibility

All benchmarks except `sysbench` support both `x86_64` and `riscv64` architectures.

These benchmarks are precompiled and packaged into the Docker image for convenience. Refer to `tools/docker/nix/Dockerfile` for details.

## Adding New Benchmarks

We recommend utilizing `Nix` when adding new benchmarks. To check if a benchmark is already available, use the [`Nix Package Search`](https://search.nixos.org/packages?channel=25.05). If a package exists in the Nix channel, you can directly use it or modify it if necessary.

If the desired benchmark is not available or cannot be easily adapted, you can add a custom `.nix` file to package it manually. Place the `.nix` files under the `test/initramfs/nix/benchmark` directory.

## Configuration Files

Configuration files required by benchmarks or regression tests should be placed in the `test/initramfs/etc` directory.

If additional configuration files or directories are needed, ensure they are appropriately packaged by updating the `initramfs.nix` file.

## Notes for Developers

- **Nix Usage**: Use `Nix` whenever possible to manage dependencies and builds for ease of maintenance and consistency.
- **Multi-Architecture Support**: Ensure new regression tests or benchmarks properly support multiple CPU architectures.
