# Maintaining the Nix Development Environment

This document is for developers who edit the Nix files in this directory.
If you only want to use the shell, read
[Using Nix for Development](../../../book/src/kernel/nix-development.md) instead.

## File organization

- The root [`flake.nix`](../../../flake.nix) declares the inputs
  and exports the development shells and the boot-stack packages
  for `x86_64-linux` and `aarch64-linux`.
- [`overlay.nix`](overlay.nix) assembles the Rust toolchain, the vDSO source,
  and the project-specific packages into a nixpkgs overlay.
- [`devshell.nix`](devshell.nix) selects the host tools
  and sets the environment variables of the shell.
- [`packages/`](packages/) contains the definitions of QEMU, GRUB, and the OVMF firmware.
- The test suites are packaged under [`test/initramfs/nix`](../../../test/initramfs/nix), not here.
  The shell only provides the `nix` command that the existing Make targets use to build them.

nixpkgs packages QEMU, GRUB, and OVMF too, but for now the shell must provide the same boot stack as the Docker image,
so each definition under `packages/` pins the versions or source revisions the Dockerfiles use
and overrides nixpkgs' patches and build settings where needed.
The header comment of each file lists its deviations from nixpkgs.

The shell sets `GRUB_MKRESCUE` to its packaged GRUB executable
so that the `iso` and `nixos` targets do not depend on `/usr/bin/grub-mkrescue`.

The host-side clients of the network benchmarks come from
[`test/initramfs/nix`](../../../test/initramfs/nix/default.nix),
the definitions that the Docker image installs with `make install_host_pkgs`.

## Dependency versions

The Rust toolchain is read from [`rust-toolchain.toml`](../../../rust-toolchain.toml),
and the shell adds `rust-analyzer` from the same nightly.
Never pin a second Rust version in the Nix expressions.

The QEMU and GRUB versions and the edk2 release used to build OVMF follow the
[OSDK Dockerfile](../../../osdk/tools/docker/Dockerfile).
The vDSO revision follows the
[kernel development Dockerfile](../docker/kernel-dev/Dockerfile).
When you bump one of these dependencies in a Dockerfile,
bump its Nix counterpart in the same change.
Nothing in CI compares the two yet, so a Dockerfile-only bump passes unnoticed.
Until such a check exists, verify the pins by hand.
If you do not have Nix installed,
update the version or revision, set the `hash` to an empty string,
and let the [Test Nix flake workflow](../../../.github/workflows/test_nix_flake.yml) run.
That first run is expected to fail with a hash mismatch.
The error prints the real hash after `got:`, so copy that value into the file.
The workflow then rebuilds the packages and boots the kernel from them.

The main nixpkgs revision must match the one pinned in
[`distro/nixpkgs.nix`](../../../distro/nixpkgs.nix) and in the
[prebuilt Nix packages Dockerfile](../docker/prebuilt-nix-packages/Dockerfile).
The "Check nixpkgs revisions" step of the
[Test Nix flake workflow](../../../.github/workflows/test_nix_flake.yml)
fails when the three pins diverge.

The `typos` version is pinned to the one in the OSDK Dockerfile
through a separate nixpkgs input, because that Dockerfile checks the spelling with a fixed release.
The other tools that the Dockerfile installs with `cargo install`
come from the main nixpkgs input and may be older or newer than the Docker versions.
The shell omits klint, because no build or check target invokes it.

## Validation

From the repository root, evaluate every exported system without touching the lock file:

```bash
nix flake check --no-build --all-systems --no-update-lock-file
```

When you change a package definition, build the boot-stack packages:

```bash
nix build .#qemu .#grub .#ovmf
```

Then reproduce the CI checks, which run the shell with a stripped-down host environment:

```bash
nix develop --ignore-environment --keep HOME --command make check
nix develop --ignore-environment --keep HOME --command make run_kernel AUTO_TEST=boot
```

CI runs the two commands above on an ARM64 runner as well,
with `ENABLE_KVM=0` because the x86-64 kernel runs under TCG there.

To check the formatting of the Nix files alone, pass their paths to the shared formatter script:

```bash
./tools/nixfmt.sh --check flake.nix tools/dev_env/nix
```
