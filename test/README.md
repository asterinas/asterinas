# Test Suites

This directory contains the testing infrastructure for Asterinas, organized into two complementary testing approaches.

## Test Types

### Initramfs-Based Tests ([`initramfs/`](initramfs/))

Tests running in a minimal initramfs environment. Best for:
- System call validation
- Core functionality testing
- Performance benchmarks

See [`initramfs/README.md`](initramfs/README.md) for details.

### NixOS-Based Tests ([`nixos/`](nixos/))

Tests running in NixOS environments. 

See [`nixos/README.md`](nixos/README.md) for details.