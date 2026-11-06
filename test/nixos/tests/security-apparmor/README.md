# AppArmor parser and policy interoperability

Build and run manually using the existing NixOS test runner:

```sh
make nixos NIXOS_TEST_SUITE=security-apparmor
make run_nixos NIXOS_TEST_SUITE=security-apparmor
```

The suite installs the tests from the same pinned AppArmor 5.0.2 source
as the parser package, preserving the upstream scripts and GPL-2.0 license.
`simple.pl` checks six original valid and invalid profile-syntax cases
using their upstream expected results.
`dirtest.sh` checks directory enumeration and invalid directory entries.
Both upstream scripts run unchanged and do not load policies into the kernel.

A separate interoperability test compiles the original `profileA` and `profileB`
inputs from `parser/tst/dirtest/gooddir`, loads them individually in reverse order,
and checks sorted queries, enforce/complain modes, and duplicate-load rejection
without changing the published policy.

These checks cover parser packaging and the supported M1 control path.
The upstream kernel regression suite requires interfaces such as policy replacement,
removal, task labels, or enforcement and is not claimed to pass on M1.
