# Maintaining the Nix Development Environment

This document is for developers who edit the Nix files in this directory.
If you only want to use the shell, read
[Using Nix for Development](../../../book/src/kernel/nix-development.md) instead.

## File organization

- The root [`flake.nix`](../../../flake.nix) declares the inputs
  and exports the development shells, boot-stack packages, and vDSO files
  for `x86_64-linux` and `aarch64-linux`.
- [`overlay.nix`](overlay.nix) assembles the Rust toolchain, the vDSO source,
  and the project-specific packages into a nixpkgs overlay.
- [`devshell.nix`](devshell.nix) selects the host tools
  and sets the environment variables of the shell.
- [`packages/`](packages/) contains the definitions of QEMU, GRUB, and the OVMF firmware.
- The test suites are packaged under [`test/initramfs/nix`](../../../test/initramfs/nix), not here.
  The shell only provides the `nix` command that the existing Make targets use to build them.

The definitions under `packages/` select the QEMU, GRUB, and OVMF versions used for development
and adjust nixpkgs' patches and build settings where needed.
Comments in each package definition explain its deviations from nixpkgs.

The shell sets `GRUB_MKRESCUE` to its packaged GRUB executable
so that the `iso` and `nixos` targets do not depend on `/usr/bin/grub-mkrescue`.

The host-side clients of the network benchmarks come from
[`test/initramfs/nix`](../../../test/initramfs/nix/default.nix),
the definitions that the Docker image installs with `make install_host_pkgs`.

## Dependency versions

The Rust toolchain is read from [`rust-toolchain.toml`](../../../rust-toolchain.toml),
and the shell adds `rust-analyzer` from the same nightly.
Never pin a second Rust version in the Nix expressions.

The GRUB version and the edk2 release used to build OVMF follow the
[OSDK Dockerfile](../../../osdk/tools/docker/Dockerfile).
When you bump one of these dependencies in that Dockerfile,
bump its Nix counterpart in the same change.
Nothing in CI compares the two yet, so a Dockerfile-only bump passes unnoticed.
Until such a check exists, verify the pins by hand.
If you do not have Nix installed,
update the version or revision, set the `hash` to an empty string,
and let the [Test Nix flake workflow](../../../.github/workflows/test_nix_flake.yml) run.
That first run is expected to fail with a hash mismatch.
The error prints the real hash after `got:`, so copy that value into the file.
The workflow then rebuilds the packages and boots the kernel from them.

To update QEMU, change its version and hash in [`packages/qemu.nix`](packages/qemu.nix),
then run `nix build .#qemu`.
The development shell and the [OSDK image](../../../osdk/tools/docker/Dockerfile) both use this definition,
so rebuild the OSDK image and the images built on it.

To update the vDSO files, change the revision and hash of `asterinas-vdso` in [`overlay.nix`](overlay.nix),
then run `nix build .#vdso`.
Rebuild the [kernel development image](../docker/kernel-dev/Dockerfile) afterward,
because it builds the same definition.

To update the main nixpkgs source, run `nix flake update nixpkgs` from the repository root,
either on a host with Nix installed or inside the project development container.
The development shell and Make-based builds both read this source from `flake.lock`.
Review the lock diff, then run the validation commands below.
If Nix reports that `nix-command` or `flakes` is disabled,
add `--extra-experimental-features 'nix-command flakes'` to the command.

The [prebuilt Nix packages image](../docker/prebuilt-nix-packages/Dockerfile)
creates its channels from the locked revision when the image is built.
Updating `flake.lock` does not change the channels in published images,
so rebuild the image to pick up the new revision.
The "Check nixpkgs source" step in the
[Test Nix flake workflow](../../../.github/workflows/test_nix_flake.yml)
fails if [`distro/nixpkgs.nix`](../../../distro/nixpkgs.nix) resolves to a different source than the flake.

`--override-input` changes only what the flake sees.
Make-based builds read `flake.lock` directly,
so update the lock itself to test a nixpkgs change across all entry points.

The `typos` version is pinned to the one in the OSDK Dockerfile
through a separate nixpkgs input, because that Dockerfile checks the spelling with a fixed release.
The other tools that the Dockerfile installs with `cargo install`
come from the main nixpkgs input and may be older or newer than the Docker versions.
The shell omits klint, because no build or check target invokes it.

## Docker builds

The OSDK image installs Nix and builds `.#qemu`.
Downstream images inherit that Nix installation and store.
The QEMU and vDSO builds accept the flake's Cachix configuration,
so Nix can download matching cached outputs instead of rebuilding them.

QEMU has a named garbage-collection root under `/nix/var/nix/gcroots/qemu`.
Preserve it when changing the prebuilt image's cleanup steps,
which remove automatic roots before building the test packages.
Do not add garbage collection after the final `initramfs_pkgs` build.
That step keeps its build dependencies, such as `stdenvNoCC`, in the store for CI.

The prebuilt Dockerfile does not install Nix,
so build it on an OSDK image from the same revision.
Use a new shared tag for the whole chain,
because the publication workflow skips tags that already exist.
Build `osdk-dev` first, then `prebuilt-nix-packages`, `kernel-dev`, and `dev`.
Publish every required platform before pointing Make or CI at the new tag.

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
