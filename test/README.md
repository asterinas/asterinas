# Test Suite Overview

This directory contains the test suites of Asterinas, including various test programs, benchmarks, syscall test suites, and necessary configuration files. The structure of the test directory is designed to be modular and flexible, supporting multiple CPU architectures and a streamlined workflow for building and running tests.

## Directory Structure

```
test/
├── src/
│   ├── apps/          # Handwritten test applications
│   ├── benchmark/     # Supported benchmark test suites
│   ├── etc/           # Configuration files
│   └── syscall/       # Syscall test suites
│       ├── ltp/       # LTP syscall test suite
│       └── gvisor/    # Gvisor syscall test suite
├── nix/
│   ├── benchmark/     # Nix expressions for `benchmark`
│   ├── syscall/       # Nix expressions for `syscall`
│   ├── apps.nix       # Nix expression for `apps`
│   └── initramfs.nix  # Nix expression for packaging initramfs
├── Makefile
└── README.md
```

---

## Building and Packaging Tests

Most tests in this directory are compiled and packaged using [Nix](https://nixos.org/), a powerful package manager. This ensures consistency and reproducibility across environments.

> **Note**: If you are adding a new test to the `apps` directory, ensure that it supports multiple architectures. Some of the existing apps lack proper architecture-specific handling.

### Syscall Test Suite - Gvisor Exception

While most tests rely on `Nix` for compilation, the `gvisor` syscall test suite currently cannot be built with `Nix`. Instead, the `gvisor` tests are compiled in the Docker image. For details, refer to `tools/docker/Dockerfile`.

### Multi-Architecture Support
The test suite supports building for multiple architectures, including `x86_64` and `riscv64`. You can specify the desired architecture by running:

```bash
make build ARCH=x86_64
# or
make build ARCH=riscv64
```

The build artifacts (initramfs) can be found in the `test/build` directory after the compilation.

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

If the desired benchmark is not available or cannot be easily adapted, you can add a custom `.nix` file to package it manually. Place the `.nix` files under the `test/nix/benchmark` directory.

## Configuration Files

Configuration files required by benchmarks or apps should be placed in the `test/src/etc` directory.

If additional configuration files or directories are needed, ensure they are appropriately packaged by updating the `initramfs.nix` file.

## Notes for Developers

- **Nix Usage**: Use `Nix` whenever possible to manage dependencies and builds for ease of maintenance and consistency.
- **Multi-Architecture Support**: Ensure new apps or benchmarks properly support multiple CPU architectures.
