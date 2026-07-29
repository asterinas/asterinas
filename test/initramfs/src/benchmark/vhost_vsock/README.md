# vhost-vsock benchmark

This suite compares the in-kernel `/dev/vhost-vsock` data path with the
`vhost-device-vsock` 0.3.0 userspace backend without adding a nested virtual
machine or VMM. The same benchmark process emulates the guest endpoint with
split virtqueues and switches only the backend control and host transports:

- `vhost` uses vhost ioctls plus an `AF_VSOCK` host stream;
- `vhost-user` uses the vhost-user protocol plus the backend's hybrid-vsock
  Unix-domain host stream.

Both backends receive the same shared `memfd`, queue size (256 entries), guest
credit window (256 KiB), packet buffers, and workload. The userspace backend is
started with a matching 256-KiB TX credit window. Warmup runs on a separate
device and connection, so no connection-specific credit or notification state
leaks into the timed transfer.

The workload follows the shape of Linux
[`tools/testing/vsock/vsock_perf.c`](https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/tree/tools/testing/vsock/vsock_perf.c):

- each result covers one transfer direction;
- setup, connection establishment, and warmup are excluded from timing;
- the byte budget and per-operation buffer size are explicit;
- throughput uses the full transfer wall time.

The benchmark also implements virtio-vsock credit updates. This lets transfers
exceed the socket buffer instead of measuring only an initial unthrottled burst.
The profile line reports packet, kick, call, and host syscall counts for
diagnosis, while the official benchmark result is throughput in Mbits/sec.

This is a backend microbenchmark, not a Kata or end-to-end QEMU benchmark. It
isolates queue processing by keeping the emulated guest, memory layout, and
workload in one process. The vhost-user backend necessarily uses its hybrid
vsock Unix socket while `/dev/vhost-vsock` uses `AF_VSOCK`, so the comparison
includes the backend's userspace scheduling and host-transport overhead.

Both comparison kernels must expose `/dev/vhost-vsock` for the in-kernel half
of the suite. A Linux kernel built with `CONFIG_VHOST_VSOCK=m` therefore needs
matching modules in the initramfs. The vhost-user half additionally requires
Unix `SCM_RIGHTS`, `memfd`, `eventfd`, and shared mappings.
