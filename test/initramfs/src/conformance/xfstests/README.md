# xfstests Conformance Suite

This directory contains the Asterinas integration for [xfstests](https://git.kernel.org/pub/scm/fs/xfs/xfstests-dev.git). Asterinas packages xfstests into initramfs, boots Asterinas in QEMU, prepares the configured filesystem, and runs the selected xfstests cases in the guest.

## Directory Structure

```
xfstests/
|-- run_xfstests.sh          # Guest-side wrapper around upstream ./check
|-- ext2/                    # Configuration for XFSTESTS_FS_TYPE=ext2
|   |-- config/
|   |   |-- build_config.mk  # Build-time image and mkfs settings
|   |   `-- xfstests.config  # Runtime options loaded by xfstests
|   |-- prepare.sh           # Guest-side setup before ./check
|   `-- run_list/
|       |-- block.list       # Tests excluded from every run
|       |-- short.list       # Default quick run list
|       `-- full.list        # Broader manual or scheduled run list
|-- tmpfs/
|   `-- ...
`-- template/                # Starting point for a new filesystem
```

Each supported filesystem has a directory named after the `XFSTESTS_FS_TYPE` value passed to the top-level `Makefile`.

## Running Tests

Run these commands from the project root inside the Asterinas development container.

```bash
# Run the default ext2 short list
make run_kernel AUTO_TEST=conformance CONFORMANCE_TEST_SUITE=xfstests

# Run the tmpfs short list
make run_kernel AUTO_TEST=conformance CONFORMANCE_TEST_SUITE=xfstests \
    XFSTESTS_FS_TYPE=tmpfs

# Run the ext2 full list
make run_kernel AUTO_TEST=conformance CONFORMANCE_TEST_SUITE=xfstests \
    XFSTESTS_FS_TYPE=ext2 \
    XFSTESTS_RUNLIST=/opt/xfstests/ext2/run_list/full.list
```

To run one or a few cases locally, create a run list under `<fs>/run_list/` and pass its guest path with `XFSTESTS_RUNLIST`:

```text
# ext2/run_list/local.list
generic/001
generic/002
```

```bash
make run_kernel AUTO_TEST=conformance CONFORMANCE_TEST_SUITE=xfstests \
    XFSTESTS_FS_TYPE=ext2 \
    XFSTESTS_RUNLIST=/opt/xfstests/ext2/run_list/local.list
```

For block-based filesystems, the build creates test and scratch disk images before booting QEMU. Their default size is 12 GiB:

```bash
make run_kernel AUTO_TEST=conformance CONFORMANCE_TEST_SUITE=xfstests \
    XFSTESTS_DISK_SIZE=2G
```

## Configuration

`run_xfstests.sh` selects `/opt/xfstests/<XFSTESTS_FS_TYPE>/`, prepares the test environment, and then invokes upstream xfstests `./check`. `./check` is the standard xfstests runner. Before calling it, Asterinas sets the xfstests `HOST_OPTIONS` environment variable to the selected `config/xfstests.config` file, so `./check` can load the filesystem type, devices, mount points, and mount options.

Filesystem directories provide these files:

- `config/build_config.mk`: Build-time settings used before QEMU starts. It controls whether xfstests needs test and scratch block devices, and which `mkfs` command formats them.
- `config/xfstests.config`: Runtime host options file loaded by `./check`. Asterinas points `HOST_OPTIONS` at this file before running xfstests. It sets variables such as `FSTYP`, `TEST_DEV`, `SCRATCH_DEV`, `TEST_DIR`, `SCRATCH_MNT`, and mount options.
- `prepare.sh`: Guest-side setup run before `./check`. It should create mount points, mount the test and scratch filesystems, and fail explicitly if setup is incomplete.
- `run_list/short.list`: Default quick run list for local use and regular CI. Keep it deterministic, reasonably fast, and useful for regression coverage.
- `run_list/full.list`: Broader run list for manual validation or scheduled CI.
- `run_list/block.list`: Tests excluded from every run for this filesystem. Use it for known hang and kernel panic (e.g., OOM).

Common `Makefile` variables:

- `XFSTESTS_FS_TYPE`: Filesystem configuration to use. Defaults to `ext2`.
- `XFSTESTS_RUNLIST`: Guest path to a run list. Defaults to `/opt/xfstests/<XFSTESTS_FS_TYPE>/run_list/short.list`.
- `XFSTESTS_DISK_SIZE`: Size of each generated block image. Defaults to `12G`.
- `XFSTESTS_TEST_DEV`: Guest test device. Defaults to `/dev/vdc`.
- `XFSTESTS_SCRATCH_DEV`: Guest scratch device. Defaults to `/dev/vdd`.

## Adding Tests

The upstream xfstests test bodies come from the Nix package. To add coverage for an existing xfstests case, add the case name, such as `generic/001`, to the target filesystem's `run_list/full.list`. Add it to `short.list` only if it is suitable for regular local and CI runs. Run the case locally before submitting the change.

If the case needs different devices, mount options, mkfs options, or setup, update the filesystem's `config/xfstests.config` or `prepare.sh`.

## Adding a New Filesystem

Copy the template and fill in the filesystem-specific configuration:

```bash
cd test/initramfs/src/conformance/xfstests
cp -r template myfs
```

Then update:

1. `myfs/config/build_config.mk` for block-device and mkfs settings.
2. `myfs/config/xfstests.config` for xfstests runtime variables.
3. `myfs/prepare.sh` for guest-side setup before `./check`.
4. `myfs/run_list/*.list` for selected and blocked tests.

Run the new filesystem with:

```bash
make run_kernel AUTO_TEST=conformance CONFORMANCE_TEST_SUITE=xfstests \
    XFSTESTS_FS_TYPE=myfs
```

If the filesystem needs tools that are not already packaged with xfstests, update `test/initramfs/nix/conformance/xfstests.nix`.
