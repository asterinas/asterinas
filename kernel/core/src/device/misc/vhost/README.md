# Writing a vhost backend

This directory provides the common part of an in-kernel vhost device:
Linux vhost configuration, access to the owner's memory, and split virtqueues.
A device backend supplies the protocol and runs the workers.
For example, a vsock backend adds guest CIDs and socket routing;
a network backend would connect its queues to a network interface.

Start with [`VhostDeviceState` and `VhostRuntime`](mod.rs),
then the [queue and descriptor-chain APIs](virtqueue.rs).
The [tests](tests.rs) contain both byte-array fixtures and real VMAR examples.
The shared wire layouts live in
[`aster_virtio::virtio_ring`](../../../../comps/virtio/src/virtio_ring.rs).
The virtio frontend and vhost use opposite sides of the same ring;
they share layouts, but keep their queue state machines separate.

## Responsibilities

| Common vhost | Device backend |
| --- | --- |
| Common ioctl parsing, owner checks, feature masks | Device-file registration and device-specific ioctls |
| Memory-table validation and GPA translation | Protocol headers, request validation and responses |
| Split-ring traversal, direct and indirect chains | Queue count and the meaning of each queue |
| Sequential access across descriptor segments | Worker creation, scheduling, batching and backpressure |
| Used-ring publication and eventfd operations | Stop/wake/join, configuration serialization and error policy |

The current facade is visible to backends under `device::misc`.
It uses `aster-core`'s VMAR, file-table, ioctl and event facilities;
it has no dependency on a KVM VM, vCPU or memory slot.
Keeping it in `aster-core` avoids exposing those crate-private facilities to other crates.
Keep a new device's protocol and transport in its own module.
Put additional shared mechanisms here only when their semantics apply across devices.

## Small device example

The following echo device has one queue and copies four readable bytes into four writable bytes.
Place it in a sibling module under `device::misc`.
The device-file adapter supplies registration and wraps `EchoDevice` in a sleeping mutex:
forward common ioctls to `ioctl()` and a device-specific start command to `start()`.
Release it in sleepable context so `Drop` can join the worker.

Configuration commands stop the worker; userspace explicitly starts it again after configuration.
`GET_VRING_BASE` therefore observes a stopped queue, and reset joins before releasing the owner.
The example signals the queue error event and exits on any protocol, copy or wait error;
it does not complete a failed request or restart automatically.
The loop handles one request per pass and yields between requests.

```rust
use super::vhost::{self, VhostDeviceConfig, VhostDeviceState, VhostVirtQueue};
use crate::{
    events::{EventFile, EventFileFlags, IoEvents, KernelEventFile},
    prelude::*,
    process::signal::{Pollable, Poller},
    thread::{Thread, kernel_thread::ThreadOptions},
    util::ioctl::{RawIoctl, dispatch_ioctl},
};

struct EchoDevice {
    common: VhostDeviceState<1>,
    worker: Option<Arc<Thread>>,
    stop: Arc<KernelEventFile>,
}

impl EchoDevice {
    fn new() -> Result<Self> {
        Ok(Self {
            common: VhostDeviceState::new(VhostDeviceConfig {
                device_features: vhost::VIRTIO_F_VERSION_1 | vhost::VIRTIO_RING_F_INDIRECT_DESC,
                backend_features: 0,
                max_queue_size: 256,
            }),
            worker: None,
            stop: KernelEventFile::from_file(&EventFile::new(0, EventFileFlags::empty()))?,
        })
    }

    fn ioctl(&mut self, raw: RawIoctl) -> Result<i32> {
        use vhost::ioctl_defs::*;

        dispatch_ioctl!(match raw {
            GetFeatures | GetBackendFeatures | SetOwner => {
                self.common.handle_ioctl(raw)
            }
            _ => {
                self.common.check_owner()?;
                self.stop();
                dispatch_ioctl!(match raw {
                    ResetOwner => {
                        self.common.reset_owner_after_quiesce();
                        Ok(0)
                    }
                    _ => {
                        self.common.handle_ioctl(raw)
                    }
                })
            }
        })
    }

    fn start(&mut self) -> Result<()> {
        if self.worker.is_some() {
            return_errno_with_message!(Errno::EBUSY, "echo worker is already started");
        }
        let mut runtime = self.common.build_runtime()?;
        let kick = runtime.queue_mut(0)?.kick_event().ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "echo worker needs a kick eventfd")
        })?;
        self.stop.consume();
        let stop = self.stop.clone();
        let vmar = runtime.vmar().clone();
        self.worker = Some(
            ThreadOptions::new(move || {
                // Reconfiguration joins this worker before invalidating its runtime.
                let queue = runtime.queue_mut(0).unwrap();
                if Self::run(queue, &kick, &stop).is_err() {
                    queue.signal_error();
                }
            })
            .vmar(vmar)
            .spawn(),
        );
        Ok(())
    }

    fn run(
        queue: &mut VhostVirtQueue,
        kick: &KernelEventFile,
        stop: &KernelEventFile,
    ) -> Result<()> {
        loop {
            let mut poller = Poller::new(None);
            kick.poll(IoEvents::IN, Some(poller.as_handle_mut()));
            if !stop
                .poll(IoEvents::IN, Some(poller.as_handle_mut()))
                .is_empty()
            {
                return Ok(());
            }
            queue.consume_kick();
            queue.disable_kick_notifications()?;
            if let Some(chain) = queue.try_pop()? {
                if chain.readable_len() != 4 || chain.writable_len() != 4 {
                    return_errno_with_message!(Errno::EINVAL, "invalid echo request size");
                }
                let mut data = [0u8; 4];
                chain.reader().read_exact(&mut data)?;
                let mut writer = chain.writer();
                writer.write_all(&data)?;
                queue.add_used(&chain, writer.bytes_written() as u32)?;
                queue.notify()?;
                Thread::yield_now();
            } else if !queue.enable_kick_notifications()? {
                poller.wait()?;
            }
        }
    }

    fn stop(&mut self) {
        if let Some(worker) = self.worker.take() {
            self.stop.signal();
            worker.join();
        }
    }
}

impl Drop for EchoDevice {
    fn drop(&mut self) {
        self.stop();
    }
}
```

The sections below explain the common API and the obligations of a real backend.

## 1. Configure the device

Create one `VhostDeviceState<Q>` per open device session.
Choose `Q`, supported feature masks and the maximum queue size from the device protocol.
Do not advertise features that the common layer or your backend cannot implement.
In particular, event-index notifications, packed rings, IOTLB translation and dirty logging
are not implemented by this common layer.

Forward ordinary common commands to `common.handle_ioctl(raw_ioctl)`.
`RawIoctl` already contains the command and userspace argument address;
there is no separate `(command, address)` overload.
Unknown commands return `ENOTTY`.
Use the typed commands in `ioctl_defs` with the existing `dispatch_ioctl!` convention
when intercepting lifecycle commands or adding your device's commands.

| Command group | Behavior |
| --- | --- |
| `GET_FEATURES`, `GET_BACKEND_FEATURES` | Report the backend-supplied masks |
| `SET_OWNER` | Capture the calling POSIX thread's VMAR |
| `SET_FEATURES`, `SET_BACKEND_FEATURES` | Check ownership and reject unsupported bits |
| `SET_MEM_TABLE` | Validate and store GPA-to-owner-address regions |
| `SET_VRING_NUM`, `SET_VRING_ADDR`, `SET_VRING_BASE` | Configure each queue |
| `SET_VRING_KICK`, `SET_VRING_CALL`, `SET_VRING_ERR` | Retain kernel eventfd handles; `fd == -1` unbinds |
| `GET_VRING_BASE` | Read the current available index; the backend must stop the affected worker first |
| `RESET_OWNER` | Intercept in the backend; common dispatch deliberately does not reset a running device |

All command names in the table have the `VHOST_` prefix.
The feature getters do not require ownership; mutations and `GET_VRING_BASE` do.
The owner is an address space, so passing the fd to another process does not transfer ownership.

`is_fully_configured()` checks for an owner, a nonempty memory table,
and a size and addresses for every queue.
Eventfds are optional at this layer.
Before starting, also validate your device-specific configuration and wakeup mechanism.
For example, a worker that only waits for kicks needs a kick eventfd.

## 2. Build the runtime and bind its worker

Call `build_runtime()` in the owner process's ioctl context.
It checks ownership and reads the used-ring headers,
so it already performs fallible memory access.
It cannot be deferred to an ordinary, unbound kernel thread.

Retain the worker thread so the backend can join it during shutdown.
The backend must prevent duplicate starts and concurrent consumers of the same queue.
Do not build two runtimes and let both process the same rings.

The worker association activates the owner VMAR when the thread is scheduled
and routes its page faults through that VMAR.
Owner-memory copies use `VmSpace::reader()` and `writer()` with fallible copies.
Do not manually activate page tables or switch to another VMAR inside the worker.
Call these APIs in sleepable thread context, with no spinlock, IRQ or preemption guard held.

The runtime keeps an `Arc<Vmar>`, not a POSIX `VmarHandle`.
It does **not** pin mappings or prevent the owner from exiting or unmapping memory.
Unmapped accesses return `EFAULT`; access with another VMAR active is rejected.
Treat a memory fault as a queue/backend failure according to your device's policy.
Do not silently retry forever or fall back to alien copies.

### Which addresses are used?

Vring addresses passed to `SET_VRING_ADDR` are already owner virtual addresses.
Descriptor buffer addresses, including indirect-table addresses, are guest physical addresses.
The memory table translates the latter into owner virtual addresses:

```text
region: GPA 0x1000..0x3000 -> owner VA 0x20000..0x22000
descriptor: GPA 0x1800, length 32 -> owner VA 0x20800, length 32
```

A descriptor can cross adjacent guest regions even when their owner addresses are disjoint.
The chain reader/writer traverses the resulting segments.
Successful address translation validates the configured ranges;
the actual copy can still fault if the owner changes its mappings.

## 3. Process a descriptor chain

Directions are from the backend's perspective:
readable descriptors contain guest input;
writable descriptors provide space for backend output.
Readable descriptors must precede writable descriptors in one chain.
For vsock, guest TX is readable and guest RX is writable.

Use `runtime.queue_mut(index)?` to obtain the queue.
`try_pop()` returns `Result<Option<VhostDescriptorChain>>`;
`reader()` and `writer()` return cursors directly.
There is no direction argument to `try_pop()` and no `read_obj()` helper.
Decode your protocol headers from the returned bytes, including their byte order.

`try_pop()` advances the available index only after generic chain validation succeeds.
After it returns a chain, the backend owns that request until completion or shutdown.
Publish it once with `add_used(&chain, written_len)` on the same queue.
The used length counts bytes written into writable descriptors, not bytes read from the guest.
For a purely readable request, it is normally zero.

`notify()` is separate so a worker can publish a batch before signaling.
The example treats any protocol or copy error as fatal to its worker.
A real worker must catch the error, call `queue.signal_error()` where appropriate,
and record/stop the failed queue or return a protocol-defined error response.
Returning `Err` alone does not signal the guest or roll back a popped request.
Copies may modify a prefix before failing;
do not assume an error means no bytes were transferred.

For a receive queue, keep pending host data in backend-owned storage
when `try_pop()` returns `None`.
Do not invent a used entry without a guest descriptor.
Bound pending data, packet lengths and work per pass according to your backend's policy;
the example checks shutdown and yields between requests.

## 4. Sleep without losing a wakeup

| Eventfd | Direction | Worker action |
| --- | --- | --- |
| kick | Guest/QEMU to backend | Register `kick_event()` with a poller and call `consume_kick()` |
| call | Backend to QEMU/guest | `notify()` honors guest interrupt suppression and signals it |
| err | Backend to userspace | `signal_error()` reports a queue failure if an err fd is bound |

All three retain `Arc<KernelEventFile>` independently of the userspace file wrapper.
The common layer does not create a worker or wait on its behalf.

Before draining, register the kick event, the backend stop event,
and any transport/pending-data event with the worker's poller.
After draining, `enable_kick_notifications()` clears suppression and rechecks the available index.
If it returns `true`, drain again; a descriptor raced with re-enabling kicks.
If it returns `false`, recheck stop and backend work before waiting.
Use the registered poller so a kick arriving between the recheck and sleep wakes the worker.
Never wait only on a stop flag that cannot wake a sleeping worker.

## 5. Stop, reconfigure and reset

Serialize the entire lifecycle with a backend control lock.
Workers must not need that lock to finish;
release any shared data lock they need before joining.
In particular, never join while holding a spinlock.

For a configuration mutation while running:

1. Check the caller's ownership with `check_owner()` before stopping anything.
2. Request stop and wake every affected worker, then join them.
3. Drop the old runtimes and any retained chains/cursors.
4. Apply the ioctl, then build a new runtime in owner context if restarting.
5. Spawn workers bound to the new runtime's VMAR.

Define what happens if configuration or restart fails;
do not resume an old runtime against partially changed configuration.
For `GET_VRING_BASE`, stop the affected queue before forwarding the getter;
the common getter only reads the index and does not stop a worker.

For `VHOST_RESET_OWNER`, perform the same stop/wake/join sequence,
then call `reset_owner_after_quiesce()` and clear device-specific state.
Device close must also stop and join workers before releasing their resources,
even if no owner ioctl context is available.

Every common configuration mutation invalidates older runtime generations.
`queue_mut()` returns `EBUSY` for a stale runtime,
but a queue reference or chain already obtained can still be in use.
Generation checking is a stale-snapshot check, **not** a synchronization barrier.
It does not replace stop/join or prevent a second worker from consuming the same ring.

## Testing a new backend

Use the existing byte-array tests for ring layout and malformed-chain cases.
Use the real-VMAR tests as a model for worker binding, cross-page copies and owner exit.
They demonstrate memory access, not a running vhost device.

A new backend should additionally demonstrate:

- A real device session: configure, start, process input/output and publish used entries.
- Protocol errors and copy faults, with a defined outcome for the popped request.
- Empty receive queues, bounded pending data and a wakeup after buffers become available.
- Stop while idle and busy, reconfiguration, `GET_VRING_BASE`, reset and close.
- No queue access from old workers after reconfiguration or reset completes.

From the repository root in the development container, run the common tests with:

```sh
cd kernel/core
cargo osdk test --kcmd-args=earlycon --qemu-args="-accel kvm"
```

Check that the named vhost tests actually ran in the serial log.
This runs the `aster-core` suite, not QEMU userspace exercising a concrete vhost backend.
Report backend integration and performance separately.
