# Test Suites

This directory contains the testing infrastructure for Asterinas.

## Test Types

### Initramfs-Based Tests ([`initramfs/`](initramfs/))

Tests running in a minimal initramfs environment. Best for:
- Linux compatibility testing
- Core functionality testing
- Performance benchmarks

See [`initramfs/README.md`](initramfs/README.md) for details.

### NixOS-Based Tests ([`nixos/`](nixos/))

Tests running in NixOS environments.

See [`nixos/README.md`](nixos/README.md) for details.

### Linux Regression Runner ([`linux_regression/`](linux_regression/))

Single-VM runner that executes [`initramfs/src/regression/`](initramfs/src/regression/)
against a pinned prebuilt `x86_64` Linux `6.16.0` kernel image under KVM
to verify Linux compatibility of the regression suite.

See [`linux_regression/README.md`](linux_regression/README.md) for details.
