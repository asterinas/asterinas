# Using Nix for Development

A Nix development shell lets you build and run Asterinas without a Docker container.
It provides the same Rust toolchain, QEMU, boot firmware, and host tools as the Docker image,
so most build and test instructions in this book work as they are.
The differences are listed at the end of this page.

## Prerequisites

You need an x86-64 Linux host with [Nix](https://nixos.org/download/) installed
and [flakes enabled](https://wiki.nixos.org/wiki/Flakes#Enabling_flakes_permanently).
An ARM64 Linux shell is also provided.
It still builds the x86-64 kernel by default, which KVM cannot run on an ARM64 host,
so use the `ENABLE_KVM=0` fallback described below.

The Makefile and the build scripts use `/bin/bash`, which does not exist on NixOS by default.
On NixOS, enable [envfs](https://github.com/Mic92/envfs) in your system configuration
and rebuild the system before entering the shell:

```nix
services.envfs.enable = true;
```

QEMU uses KVM by default, so `/dev/kvm` must exist and your user must be allowed to open it.
If either is not the case,
run `make run_kernel ENABLE_KVM=0`
to fall back to software emulation, which is much slower.

## Enter the development shell

Clone the repository as described in [Getting Started](../kernel/),
then enter the development shell from the root of the checkout:

```bash
nix develop
```

The first run downloads the dependencies
and builds the packages that no public binary cache provides,
such as QEMU and the firmware.
This can take a long time.

Inside the shell, the Make targets work as they do in the Docker container.
Build and run Asterinas with the same commands as in [Getting Started](../kernel/#getting-started),
and see [Advanced Build and Test Instructions](advanced-instructions.md) for the test targets.

## Editors and direnv

The shell includes `rust-analyzer` from the same nightly as the Rust toolchain.
Start your editor from inside the shell
so that it inherits the toolchain and the `VDSO_LIBRARY_DIR` variable,
which `rust-analyzer` needs in order to check the kernel crate.
For example, if VS Code is installed on your host:

```bash
nix develop
code .
```

If you use [direnv](https://direnv.net/),
the `.envrc` at the repository root activates the shell whenever you enter the checkout.
Run `direnv allow` once to approve it.

## Differences from the Docker environment

The Docker image already contains the test suites and benchmarks prebuilt.
The shell builds them on demand instead.
The first time you run a test target, the initramfs build fetches the selected tests
and their runtime dependencies with Nix, which adds time to that first run.

The gVisor syscall tests are built with Bazel inside the Docker image,
together with a set of shared libraries that the shell does not provide.
Use the Docker environment to run them.

Projects created by `cargo osdk new` and the TDX scheme of the OSDK test suite
contain firmware paths specific to the Docker image.
They do not work in the shell unless you edit those paths yourself.

The shell points `OVMF_DIR` at the firmware in the Nix store.
If you override it, choose a path without spaces,
because the shared QEMU scripts do not quote the value of `OVMF_DIR`.
