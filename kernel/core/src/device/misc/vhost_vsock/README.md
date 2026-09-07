# vhost-vsock backend

`/dev/vhost-vsock` connects userspace-owned split queues to AF_VSOCK stream
sockets. It consumes the [common vhost layer](../vhost/README.md), including
owner checks, address translation, descriptor cursors, eventfds and used rings.
The worker is bound to the owner's VMAR. No KVM VM or memory slot is required.

Configure both queues and their kick eventfds with the common vhost ioctls,
assign a unique guest CID with `VHOST_VSOCK_SET_GUEST_CID`, then use
`VHOST_VSOCK_SET_RUNNING` to start. Queue 0 receives host packets; queue 1
transmits guest packets. CIDs 0 through 2, the wildcard CID, and CIDs outside
32 bits are rejected. CID reservations last until reassignment, reset or close.
This initial backend supports host operation when no virtio-vsock frontend
is present; starting it with an active frontend returns `EOPNOTSUPP`.

The backend supports stream packets, indirect descriptors and `VERSION_1`.
It does not advertise event-index, packed rings, logging, IOTLB, seqpacket or
zerocopy features. The guest transport event queue is managed by userspace.
Guest TX completions have length zero. RX may span multiple writable segments;
large stream payloads are split across guest buffers with a header in each.
All guest inputs and owner copies remain fallible.

A sleeping control mutex serializes device ioctls. Stop and reconfiguration
wake and join the worker while retaining connections and accepted packets.
A stopped CID remains routable and accepts bounded pending data. Successful
configuration changes restart a previously running device; `GET_VRING_BASE`
leaves it stopped. Failed configuration leaves it stopped until the owner
repairs the configuration and starts it. Reset and close also discard pending
packets, reset connections and release the CID reservation.
Memory mappings are not pinned and can disappear while the worker runs.

Host packets have a 64 KiB payload limit. Data uses at most 256 packet slots
and 1 MiB including headers; control packets have 64 additional slots. Sending
data reserves capacity before copying from userspace, so backpressure cannot
consume a caller's input before returning `EAGAIN`. RX completion wakes host
writers, and a send reservation cannot survive reset or a fatal generation change.
The worker checks for stop between requests and yields after bounded batches.

Malformed queues, owner-memory faults and control-queue exhaustion fail the
endpoint: the worker signals the configured queue error eventfds, discards
pending packets and resets affected sockets. Control exhaustion is fatal
because some stream control operations have no retry state; dropping them
would lose shutdown or credit notifications. This is an explicit bounded
resource policy, rather than Linux's potentially unbounded control queuing.
If exhaustion occurs while the worker is stopped, restarting first clears
the failed endpoint and resets its old connections. The owner must address
the error and restart the device. No failed copy is
reported as a successful used entry, including a copy that wrote a prefix.

The lock order is socket table, connection state, then backend pending state.
The CID registry is only held to clone an endpoint reference. Owner-memory
copies, transport callbacks and worker joins run without either backend
spinlock. The worker never takes the device control mutex. Applications retain
normal connection refusal and timeout behavior; no port receives special
connection retries or reset suppression.

Kernel tests cover CID reservation, packet fragmentation and pending capacity.
The `network/vhost_vsock` regression configures a real device session, checks
indirect RX and zero-length TX completions, transfers stream data and exercises
reset/refusal. These checks are separate from QEMU/TCG running Kata; that
integration also requires mount propagation and userspace configuration.

Protocol references:
[Linux v6.18 backend](https://github.com/torvalds/linux/blob/v6.18/drivers/vhost/vsock.c)
and [VIRTIO 1.2](https://docs.oasis-open.org/virtio/virtio/v1.2/virtio-v1.2.html).
