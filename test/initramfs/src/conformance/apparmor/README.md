# AppArmor kernel regression tests

The AppArmor 5.0.2 upstream regression directory is packaged from the same
source as the parser. The runner invokes the original `e2e.sh` twice and uses
the original `introspect.c` helper for locally orchestrated lifecycle tests.

Run in the Asterinas development environment:

```sh
make run_kernel AUTO_TEST=conformance CONFORMANCE_TEST_SUITE=apparmor CMDLINE="lsm=apparmor,yama"
```

The default selection runs the e2e, lifecycle, and ABI v5 storage tests. Set
`CONFORMANCE_TEST_SELECTOR` to `e2e`, `lifecycle`, or `abi-v5` to select the
guest tests separately. Every selection also runs the in-kernel ABI field tests,
including the range and truncation checks for a wrapped `rlimits` array.

The lifecycle checks attachment matching, task inheritance, current-label
queries, removal, and reloading. A profile containing file rules is saved but
does not attach to tasks. The full upstream `introspect.sh` requires runtime
support for file rules.

The storage tests compare the process labels for the same executable with and
without a capability rule. They verify that the saved rule profile retains its
declared mode and does not attach. The Nix package compiles the upstream `curl`,
`tinyproxy`, and `proftpd` profiles for ABI v5. The guest loads each profile,
checks its mode and original bytes, and removes it.
