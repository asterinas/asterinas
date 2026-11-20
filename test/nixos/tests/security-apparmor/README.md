# AppArmor policy-management regression

Run the selected AppArmor 5.0.2 regression through the existing NixOS test driver:

```sh
make nixos NIXOS_TEST_SUITE=security-apparmor
make run_nixos NIXOS_TEST_SUITE=security-apparmor
```

The suite uses the original `tests/regression/apparmor/e2e.sh` and its helpers
from the same pinned source as the parser, preserving the scripts, assertions,
and GPL-2.0 license.
It compiles an empty profile, loads it, checks the profile list,
compares the cached policy with `raw_data`, and removes the profile.
Running the script twice also exercises loading the same profile after removal.
Policy binaries are generated at runtime.

The cache-directory lookup uses libapparmor's `/proc/self/fd` path handling.
This also exercises the procfs symlink-size fix needed by the parser.
Without that fix, a zero-length `readlinkat` buffer can make the lookup fail.

`uservars.inc` supplies the paths supported by the upstream harness.
The runner checks `/bin/true` first because a missing helper can cause the
harness to skip profile generation and compare hashes of empty inputs.
Bash tracing records the assertions that actually execute.
The profile-list assertion is conditional on the file's existence;
an exit status alone does not establish that it ran.
The upstream hash pipelines can also hide read failures when both inputs are
empty, so check the trace for nonempty hashes before claiming readback coverage.
Removal checks the parser's return status, not the post-removal listing
or an already open `raw_data` descriptor.

This suite is listed in the existing NixOS CI matrix for suite changes,
manual runs, and scheduled runs; kernel-only PR changes do not trigger it.
It does not run the parser's error, syntax, directory, or cache test suites.
It also does not cover malformed binary input, capability changes,
duplicate loads, mode reporting, storage limits, or concurrency.
Task labels and access enforcement are outside the implemented functionality.
