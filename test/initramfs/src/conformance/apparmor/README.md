# AppArmor kernel regression tests

The AppArmor 5.0.2 upstream regression directory is packaged from the same
source as the parser. The runner invokes the original `e2e.sh` twice and uses
the original `introspect.c` helper for locally orchestrated lifecycle tests.

Run in the Asterinas development environment:

```sh
make run_kernel AUTO_TEST=conformance CONFORMANCE_TEST_SUITE=apparmor CMDLINE="lsm=apparmor,yama"
```

The default selection runs both e2e and lifecycle tests. Set
`CONFORMANCE_TEST_SELECTOR` to `e2e` or `lifecycle` to run either separately.
The lifecycle checks attachment matching, task inheritance and current-label
queries, including removal and reloading. Profiles containing file rules are
rejected until file-rule enforcement is implemented. The full upstream
`introspect.sh` generates file rules and is therefore not selected.
