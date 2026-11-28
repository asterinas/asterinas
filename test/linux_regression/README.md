# Linux Regression Test Runner

Run the [Asterinas regression suite](../initramfs/src/regression/) on the pinned prebuilt `x86_64` Linux `6.16.0` kernel image under QEMU/KVM.

## Usage

On an `x86_64` Linux host with Nix and read/write access to `/dev/kvm`, run from the repository root:

```bash
test/linux_regression/run.sh --timeout 15m --log-dir /tmp/linux-regression
```

Userspace output streams to the terminal.
Temporary data disks and QEMU's root snapshot overlay live in a scratch directory and are removed on exit.
Passing `--log-dir DIR` copies `userspace.log` and `kernel.log` to `DIR` on both success and failure.

To run inside the Asterinas dev Docker image on a host with KVM (create the host log directory first so Docker does not create it owned by `root`):

```bash
mkdir -p /tmp/linux-regression
docker run --rm --device=/dev/kvm -v "$PWD:/work:ro" -w /work \
    -v /tmp/linux-regression:/logs "asterinas/dev:$(cat DOCKER_IMAGE_VERSION)" \
    test/linux_regression/run.sh --log-dir /logs
```

## Blocklist

List tests that fail on the pinned kernel image in [`blocklist`](blocklist),
one executable path per line relative to `/test` in the guest,
with a `#` comment explaining why.
At build time, [`apply_blocklist.sh`](apply_blocklist.sh) validates each entry (rejecting `run_test.sh` and `run_regression_test.sh`) and replaces it with a skip stub.
To validate `blocklist` edits without booting the VM, run `nix-build --no-out-link test/linux_regression/default.nix`.
Update renamed paths and remove entries once a test is deleted or passes.
