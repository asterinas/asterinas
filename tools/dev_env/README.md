# Development environments

This directory contains the Docker and Nix configurations for Asterinas development.

- [`docker/`](docker/) holds the Dockerfiles of the development images,
  with a README on building and publishing them.
- [`nix/`](nix/) holds the Nix development shell and its packages,
  with a README on maintaining them.

The Docker images build on `asterinas/osdk-dev`.
That image also serves other OSDK-based kernels,
so its configuration stays in [osdk/tools/docker](../../osdk/tools/docker/README.md).

Both environments pin the same versions of QEMU, GRUB, edk2, the vDSO, and nixpkgs.
When you bump one of these in a Dockerfile, bump the matching Nix package as well.
