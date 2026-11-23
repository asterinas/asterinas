# Perf event acceptance

Select `perf_event_open01` from the LTP release pinned in
[`ltp.nix`](../../../nix/conformance/ltp.nix) using the existing conformance selector.

```sh
make kernel TARGET_ARCH=x86_64 ENABLE_KVM=1 ENABLE_CONFORMANCE_TEST=true \
    CONFORMANCE_TEST_SUITE=ltp CONFORMANCE_TEST_SELECTOR=perf_event_open01
```

Boot the image under KVM with a hardware PMU exposed, for example with QEMU's
`-cpu host,pmu=on`.
From the guest's root shell, invoke the standard LTP runner with verbose output.

```sh
KIRK_VERBOSE=1 /opt/ltp/run_ltp_test.sh
```

Check the test output for a `TPASS` for `PERF_COUNT_HW_INSTRUCTIONS`.
LTP reports `TCONF` for other unavailable events, such as cache misses and software clocks.
This check complements the [`perf stat` matrix](../../regression/io/perf_event_open/README.md).
