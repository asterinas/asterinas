# Asterinas AppArmor Tools

`compile-policy.sh` invokes the Linux `apparmor_parser` userspace compiler and
emits the binary policy stream accepted by Asterinas AppArmor securityfs policy
files.

```sh
tools/apparmor/compile-policy.sh ./profile.apparmor ./profile.bin
tools/apparmor/inspect-binary-policy.py ./profile.bin
```

Inside a running Asterinas system with AppArmor selected as the major LSM, load
the compiled policy through securityfs:

```sh
mount -t securityfs none /sys/kernel/security
cat ./profile.bin > /sys/kernel/security/apparmor/.replace
```

The default feature ABI used by `compile-policy.sh` mirrors the currently
implemented Asterinas AppArmor kernel surface. Pass `-f FEATURES` or set
`ASTERINAS_APPARMOR_FEATURES` to compile against a feature file exported from a
running kernel.

## Runtime Smoke Test

Run the smoke test through the Asterinas initramfs harness:

```sh
make run_kernel AUTO_TEST=apparmor
```

This compiles the smoke policy with `apparmor_parser`, packages the policy and
test script into initramfs, boots with `security=apparmor`, and checks for
`apparmor smoke: passed` in `qemu.log`.

You can also compile the smoke policy manually on a Linux host with
`apparmor_parser` installed:

```sh
tools/apparmor/compile-policy.sh \
    tools/apparmor/smoke-profile.apparmor \
    ./smoke-profile.bin
```

Copy `smoke-profile.bin` and `run-smoke-test.sh` into an Asterinas guest booted
with AppArmor selected, then run:

```sh
sh ./run-smoke-test.sh ./smoke-profile.bin
```

The smoke test mounts `securityfs` if needed, writes the binary policy to
`/sys/kernel/security/apparmor/.replace`, verifies `/proc/self/attr/exec`
on-exec transitions and `/proc/self/attr/prev`, switches the test shell through
the temporary `/proc/sys/kernel/apparmor/current` control file, then verifies
that a permitted read succeeds and an explicitly denied read fails.
