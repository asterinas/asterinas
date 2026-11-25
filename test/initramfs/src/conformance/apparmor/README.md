# AppArmor kernel regression tests

The AppArmor 5.0.2 upstream regression directory is packaged from the same
source as the parser. The M1 runner selects the original `e2e.sh` and runs it
twice in one guest, retaining its assertions and checking its results.

Run in the Asterinas development environment:

```sh
make run_kernel AUTO_TEST=conformance CONFORMANCE_TEST_SUITE=apparmor CMDLINE="lsm=apparmor,yama"
```

Only the e2e script is selected in M1. The other packaged scripts and their
helpers require additional functionality and dependencies and are not claimed
to run. The e2e test exercises empty-profile loading, querying, raw-data
reading and removal; it does not verify policy enforcement or malformed ABI.
`CONFORMANCE_TEST_SELECTOR` may be empty, `e2e`, or `e2e.sh`.
