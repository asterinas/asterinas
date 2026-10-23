# Development environments

This directory contains the Docker and Nix configurations for Asterinas development.

- [`docker/`](docker/) holds the Dockerfiles of the development images,
  with a README on building and publishing them.
- [`nix/`](nix/) holds the Nix development shell and its packages,
  with a README on maintaining them.

The Docker images build on `asterinas/osdk-dev`.
That image also serves other OSDK-based kernels,
so its configuration stays in [osdk/tools/docker](../../osdk/tools/docker/README.md).

The Docker images build QEMU and the vDSO files from the Nix definitions
and read nixpkgs from `flake.lock`, so those versions have one source.
GRUB and edk2 are still pinned in both the OSDK Dockerfile and the Nix packages.
When you bump either of them in the Dockerfile, bump the matching Nix package as well.
