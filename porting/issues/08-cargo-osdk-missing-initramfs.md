# Issue 08: `cargo osdk build` fails with missing initramfs

## Symptom

```text
[Error]: Cannot canonicalize path `test/initramfs/build/initramfs.cpio.gz`: No such file or directory
```

## Cause

OSDK's default configuration expects a pre-built initramfs. The real initramfs
is normally produced by the Nix build environment, which is not installed in
this bring-up environment.

## Fix

Create a placeholder initramfs so the build can continue:

```bash
mkdir -p test/initramfs/build
touch test/initramfs/build/initramfs.cpio.gz
```

> This is only a build workaround. The resulting kernel image has no real
> userspace initramfs content, so user-space programs will not run.

## Verification

Re-run the build:

```bash
cargo osdk build --scheme riscv --target-arch riscv64
```

The canonicalize error should be gone and compilation should continue past the
initramfs check.
