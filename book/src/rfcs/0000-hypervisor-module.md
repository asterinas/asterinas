# RFC-0000: Hypervisor Module

* Status: Draft
* Pull request: https://github.com/asterinas/asterinas/pull/3417
* Date submitted: 2026-06-21
* Date approved: YYYY-MM-DD

## Summary

This RFC proposes extending OSTD with hypervisor support. These OSTD facilities
allow `kernel/` to safely implement a complete hypervisor, including a
KVM-compatible control plane, while keeping privileged virtualization mechanisms
inside OSTD. The initial implementation targets Intel VMX with EPT on x86.

## Motivation

Asterinas needs first-class virtualization support to serve as a practical host
OS for virtualized workloads, from QEMU-managed guests to secure-container
architectures such as Kata Containers. In this role, virtualization is a core
host capability: it lets Asterinas run unmodified guest kernels, isolate
container workloads with hardware assistance, and interoperate with existing VMM
and container ecosystems.

Supporting virtualization requires extending the framekernel boundary with
care. A malformed VMCS, a bad EPT mapping, or an incorrectly restored host
register can break the soundness semantics of the host kernel. Therefore, this
RFC focuses on what kind of OSTD hypervisor layer Asterinas needs:
it must provide soundness by encapsulating privileged virtualization mechanisms,
minimality by keeping the trusted surface small, and expressiveness by giving
`kernel/` enough safe operations to build a complete hypervisor above it.

## Design

### Layering

The hypervisor stack is split into three layers.

The userspace VMM owns VM policy and device models that do not need to be in
the kernel: loading a Linux image, choosing the guest memory layout, handling
PIO or MMIO exits such as UART, and integrating with higher-level container
runtimes.

The Asterinas kernel owns the KVM-compatible file-descriptor ABI and kernel
policy. It implements `/dev/kvm`, VM file descriptors, vCPU file descriptors,
the `kvm_run` shared page, memory-slot registration, register ioctls, basic
LAPIC and I/O APIC emulation, and conversion between KVM structs and OSTD
types. This code lives under `kernel/src/device/hypervisor/` and remains safe
Rust.

OSTD owns the virtualization mechanism. It initializes VMX, manages VMCS and
EPT, saves and restores host-only CPU state, enters and exits guest mode, and
returns a small `GuestExitInfo` record to the kernel. This code lives under
`ostd/src/vm/` and `ostd/src/arch/x86/vm/`.

This split mirrors the existing user-mode design in Asterinas. The kernel can
decide what to run and how to handle exits, but the transition that could
affect host CPU state is mediated by OSTD.

### OSTD Interface Design

The OSTD hypervisor interface is centered on three safe abstractions.
`GuestPhysMemSpace` represents guest physical memory and owns the EPT-backed
translation from guest physical addresses to host frames. `GuestContext`
represents the per-vCPU architectural state that must survive across VM exits.
`GuestMode` is the execution object that enters guest mode, handles the
VM-entry/VM-exit state transition, and returns exits to the kernel. Together,
these types expose the mechanisms needed by the safe kernel hypervisor layer
while keeping VMX, VMCS, and EPT internals inside OSTD.

#### GuestPhysMemSpace

`GuestPhysMemSpace` represents a guest physical address space backed by EPT.

Its public interface is:

```rust
// ostd/src/vm/gpm_space.rs

/// Manages the guest physical memory space of a VM.
///
/// This type owns the EPT page table that maps guest physical addresses to
/// host physical frames. One `GuestPhysMemSpace` can be reused by multiple
/// vCPUs in the same VM by passing a reference to
/// [`super::GuestMode::execute`]. The kernel is responsible for pairing each
/// vCPU with the guest physical memory space that belongs to its VM.
///
/// Internally, this type reuses [`PageTable`] with [`EptPtConfig`] to manage
/// EPT mappings. It also records memory slots so a guest physical range can be
/// translated back to the userspace virtual range that backs it.
pub struct GuestPhysMemSpace;

impl GuestPhysMemSpace {
    /// Creates a new guest physical memory space.
    pub fn new() -> Self;

    /// Installs or removes a userspace-backed guest memory slot.
    ///
    /// `slot` identifies the memory slot to update. If `memory_size` is zero,
    /// this method removes the slot and its EPT mappings. Otherwise, it maps
    /// `frames` into the guest physical range starting at `guest_start` with
    /// the supplied page properties, and records the corresponding
    /// `userspace_start` so the range can later be accessed by
    /// [`Self::reader`].
    ///
    /// The backing frames are accepted as [`UFrame`]s. This typed boundary
    /// keeps safe kernel code from mapping arbitrary host-sensitive typed
    /// frames into guest memory, which is part of preserving kernel memory
    /// safety. The caller is still responsible for ensuring that the supplied
    /// frames are the frames backing the userspace range described by
    /// `userspace_start`.
    pub fn set_memory_region(
        &self,
        slot: u32,
        userspace_start: Vaddr,
        guest_start: Gpaddr,
        memory_size: usize,
        frames: Vec<UFrame>,
        prop: PageProperty,
    ) -> Result<()>;

    /// Returns a reader for a userspace-backed guest physical range.
    ///
    /// The `gpa` argument names a guest physical address. This method uses the
    /// recorded memory slots to translate the requested guest physical range
    /// back to the userspace virtual address range that backs it, then reuses
    /// [`VmReader`] to access that userspace memory.
    pub fn reader(&self, gpa: Gpaddr, len: usize) -> Result<VmReader<'_, Fallible>>;
}
```

**Soundness.**

The most important design choice is that guest RAM is installed through
`set_memory_region`, whose backing frames are `UFrame`s. EPT maps guest
physical addresses to host physical frames, and a bad mapping could let the
guest overwrite kernel code, page tables, heap objects, or frame metadata.
Requiring `UFrame` means guest memory can only be backed by untyped frames,
which do not host Rust objects and are accessed through `VmReader`/`VmWriter`
rather than Rust references.

`GuestPhysMemSpace` keeps the EPT page table inside OSTD. The safe kernel
provides memory-slot inputs and passes the `GuestPhysMemSpace` to
`GuestMode::execute`. It does not receive raw EPT pointers, EPT entries, or
page-table pages.

Memory slots record the KVM userspace address to guest physical address
relationship. `GuestPhysMemSpace::reader` uses the slot metadata to translate a
guest physical range back to the userspace backing range and returns a fallible
`VmReader`. This keeps the same safety rule used for normal user memory: the
kernel does not create Rust references to memory that userspace or the guest
can change asynchronously.

**Minimality.**

`GuestPhysMemSpace` only encapsulates the sensitive EPT page table state. The
upper layer decides which guest physical ranges should be mapped to which
backing frames; OSTD only owns the mechanism for installing and protecting
those mappings.

**Expressiveness.**

`set_memory_region` is expressive enough to map guest physical ranges to host
physical frames and to remove those mappings when a slot is no longer valid.
The `reader` interface lets the kernel fetch data from arbitrary guest physical
addresses through the `GuestPhysMemSpace` without exposing EPT internals.

#### GuestContext

`GuestContext` is the per-vCPU state object. It stores the guest-visible register
state, the vCPU run state, and the VMCS object owned by OSTD.

Its public interface is:

```rust
// ostd/src/arch/x86/vm/context.rs

/// Stores the execution context and run state of a guest vCPU.
///
/// The kernel uses it to configure the vCPU-visible context, including
/// general-purpose registers, special registers, MSRs, CPUID leaves, and
/// topology.
///
/// OSTD uses it to emulate guest instructions and to provide
/// [`crate::vm::GuestMode`] with the state needed to run the vCPU. Before
/// entering the vCPU, `GuestMode` loads the context into hardware. After a
/// VM exit, `GuestMode` synchronizes the hardware vCPU state back into this
/// context.
///
/// Setters on this type preserve internal context consistency. For example,
/// updating `CR0` or `EFER` keeps `EFER.LMA` consistent with `EFER.LME` and
/// `CR0.PG`. They do not prove that every guest-supplied value is
/// architecturally useful or bootable. The kernel remains responsible for
/// providing sensible `RIP`, general-purpose register, segment, control
/// register, `MSR`, and `CPUID` values for the guest it intends to run.
pub struct GuestContext;

impl GuestContext {
    /// Creates a guest vCPU context.
    ///
    /// The bootstrap vCPU, whose ID is zero, starts in the runnable state.
    /// Other vCPUs start in wait-for-SIPI state and become runnable after
    /// [`Self::receive_sipi`] accepts a startup vector.
    pub fn new(id: u32) -> Result<Self>;

    /// Moves an AP vCPU from wait-for-SIPI state to runnable state.
    ///
    /// The startup vector is used to rebuild the vCPU's real-mode startup
    /// state. Calling this method for a vCPU that is not waiting for SIPI has
    /// no effect.
    pub fn receive_sipi(&mut self, vector: u8);

    /// Returns the guest general-purpose register state.
    pub fn regs(&self) -> VcpuRegs;

    /// Replaces the guest general-purpose register state.
    ///
    /// This method stores the values as guest-visible state. The caller is
    /// responsible for choosing register values that make sense for the guest
    /// execution mode and entry point.
    pub fn set_regs(&mut self, regs: VcpuRegs);

    /// Returns the guest special-register state.
    ///
    /// The returned state contains the guest-visible control-register values,
    /// not the VMX-adjusted hardware values used internally for VM entry.
    pub fn sregs(&self) -> VcpuSregs;

    /// Replaces the guest special-register state.
    ///
    /// This method keeps derived context fields consistent with the supplied
    /// special registers. It updates VMX control-register shadows, synchronizes
    /// EFER state, mirrors FS/GS bases into the corresponding MSR state, and
    /// sanitizes the APIC base for this vCPU. The caller remains responsible
    /// for providing architecturally valid guest state.
    pub fn set_sregs(&mut self, sregs: VcpuSregs);

    /// Returns a guest general-purpose register by VMX register index.
    ///
    /// Invalid register indexes return zero.
    pub fn gpr(&self, index: u8) -> u64;

    /// Updates a guest general-purpose register by VMX register index.
    ///
    /// The `width_byte` argument controls whether the low 1, 2, 4, or 8 bytes
    /// are updated. Invalid register indexes are ignored. The caller is
    /// responsible for using an index and width that match the emulated guest
    /// instruction.
    pub fn set_gpr(&mut self, index: u8, width_byte: u8, value: u64);

    /// Advances the guest instruction pointer.
    ///
    /// The caller is responsible for passing the length of the instruction
    /// that has actually been consumed or emulated.
    pub fn advance_rip(&mut self, len: u64);

    /// Returns the guest instruction pointer.
    pub fn rip(&self) -> u64;

    /// Returns whether the guest vCPU is currently running.
    pub fn is_running(&self) -> bool;

    /// Returns the guest-visible TSC value.
    pub fn guest_tsc(&self) -> u64;

    /// Returns the guest-visible value of a supported MSR.
    ///
    /// Unsupported MSR indexes return `None`.
    pub fn read_msr(&self, index: u32) -> Option<u64>;

    /// Sets the guest-visible value of a supported MSR.
    ///
    /// Returns `false` if the MSR index is not supported. Supported MSRs are
    /// stored in the context and may update derived state such as TSC offset,
    /// TSC deadline, APIC base, or EFER.LMA. The caller remains responsible
    /// for choosing MSR values that are meaningful for the guest.
    pub fn write_msr(&mut self, index: u32, value: u64) -> bool;

    /// Sets the CPUID entries visible to this vCPU.
    ///
    /// The caller remains responsible for choosing CPUID values that are
    /// meaningful for the guest.
    pub fn set_cpuid_entries(&mut self, entries: Vec<GuestCpuidEntry>);

    /// Returns the CPUID result visible to this vCPU.
    ///
    /// If no configured entry matches the requested function and index, this
    /// method returns a zeroed CPUID entry.
    pub fn cpuid_result(&self, function: u32, index: u32) -> GuestCpuidEntry;
}

impl Default for GuestContext;

/// Guest general purpose registers
///
/// This structure represents the guest CPU's general purpose registers
/// that need to be saved/restored during VM entry/exit.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct VcpuRegs {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
}

/// Guest special register state.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct VcpuSregs {
    pub cs: VcpuSegment,
    pub ds: VcpuSegment,
    pub es: VcpuSegment,
    pub fs: VcpuSegment,
    pub gs: VcpuSegment,
    pub ss: VcpuSegment,
    pub tr: VcpuSegment,
    pub ldt: VcpuSegment,
    pub gdt: VcpuDtable,
    pub idt: VcpuDtable,
    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub efer: u64,
    pub apic_base: u64,
    pub interrupt_bitmap: [u64; 4],
}

/// Guest segment register state.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct VcpuSegment {
    pub base: u64,
    pub limit: u32,
    pub selector: u16,
    pub type_: u8,
    pub present: u8,
    pub dpl: u8,
    pub db: u8,
    pub s: u8,
    pub l: u8,
    pub g: u8,
    pub avl: u8,
    pub unusable: u8,
    pub padding: u8,
}

/// Guest descriptor table state.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct VcpuDtable {
    pub base: u64,
    pub limit: u16,
    pub padding: [u16; 3],
}

/// A CPUID result entry visible to a guest vCPU.
#[derive(Clone, Copy, Debug)]
pub struct GuestCpuidEntry {
    /// CPUID function, i.e., input `EAX`.
    pub function: u32,
    /// CPUID index/subleaf, i.e., input `ECX`.
    pub index: u32,
    /// KVM-compatible flags describing how the entry should be matched.
    pub flags: u32,
    /// Output `EAX`.
    pub eax: u32,
    /// Output `EBX`.
    pub ebx: u32,
    /// Output `ECX`.
    pub ecx: u32,
    /// Output `EDX`.
    pub edx: u32,
}
```

Its role is broader than a plain KVM register dump. It contains:

* general-purpose registers in `VcpuRegs`;
* special registers, segment state, descriptor tables, EFER, APIC base, and
  interrupt bitmap in `VcpuSregs`;
* emulated guest-visible MSRs in `VcpuMsrs`;
* FPU/SIMD state in `FpuContext`;
* vCPU lifecycle state such as `Runnable`, `Running`, and `WaitForSipi`;
* VMX control-register state in `VcpuControlRegisters`;
* the OSTD-owned `Vmcs`.

**Soundness.**

The register setters accept guest-provided values and perform a small amount
of consistency maintenance for guest system registers. For example, EFER.LMA is
synchronized from EFER.LME and CR0.PG, and CR0/CR4 values are adjusted against
VMX fixed-bit MSRs before they are used for VM entry. These operations help the
kernel construct a valid guest state, but they do not guarantee that every
guest-supplied state is architecturally valid.

The soundness risk is not that a guest may receive bad register state. Bad
guest state can make the guest crash, triple-fault, or fail VM entry. The real
risk is confusing guest state with host state in VMX root mode. `GuestContext`
does not expose interfaces for setting host-sensitive registers or host-only
VMCS fields; those details remain `pub(crate)` inside OSTD.

**Minimality.**

`GuestContext` centralizes the vCPU data that OSTD must understand when
switching into VMX non-root mode. To the kernel, it is a minimal vCPU state
interface for configuring and running a guest. Additional device-level vCPU
state, such as local APIC state, remains in the kernel or userspace VMM layer.

**Expressiveness.**

`GuestContext` exposes enough structure for the kernel to configure key guest
registers such as RIP, control registers, and MSRs. This lets the kernel build
vCPUs in real mode, protected mode, or long mode, and update guest state while
emulating exits such as MMIO accesses.

#### GuestMode

`GuestMode` is the OSTD execution object that performs VM entry and returns VM
exits. The public interfaces are:

````rust
// ostd/src/vm/mod.rs

/// Runs guest vCPU code in an isolated guest execution mode.
///
/// `GuestMode` is the OSTD-side execution object for a guest vCPU. It borrows
/// the vCPU context and the kernel-provided interrupt and timer policy ports,
/// then enters guest execution until a VM exit must be handled outside OSTD.
///
/// On x86, the implementation uses VMX to enter VMX non-root mode. The CPU
/// executes the code described by [`GuestContext`] while memory accesses are
/// translated through the EPT owned by the [`GuestPhysMemSpace`] passed to
/// [`Self::execute`]. Provided that the EPT maps only guest-owned memory and
/// selected device ranges, guest code cannot directly access host memory
/// outside those mappings. This protects kernel memory safety from direct
/// guest memory access.
///
/// VMCS controls force the CPU to leave VMX non-root mode on events that must
/// be handled by the host, such as external interrupts, EPT violations, I/O
/// instructions, or selected control-register and MSR accesses. OSTD handles
/// exits that belong to the low-level CPU contract, such as CPUID, CR access,
/// and MSR read/write emulation. Other exits are returned to the kernel as
/// [`GuestExitInfo`] so the kernel can emulate devices, forward events to
/// userspace, or stop the vCPU. This VM-exit boundary prevents guest code from
/// escaping guest execution and running arbitrary host control flow.
///
/// Here is a sample code on how to use `GuestMode`.
///
/// ```no_run
/// use ostd::{
///     arch::vm::GuestContext,
///     prelude::*,
///     sync::{Mutex, SpinLock},
///     vm::{GuestInterruptPort, GuestMode, GuestPhysMemSpace, GuestTimerPort},
/// };
///
/// fn run_guest(
///     context: &Mutex<GuestContext>,
///     interrupt_port: &SpinLock<dyn GuestInterruptPort>,
///     timer_port: &SpinLock<dyn GuestTimerPort>,
///     guest_mem: &GuestPhysMemSpace,
/// ) -> Result<()> {
///     let mut guest_mode =
///         GuestMode::new(context, interrupt_port, timer_port);
///
///     loop {
///         let _exit_info = guest_mode.execute(guest_mem)?;
///         todo!("handle the userspace-visible VM exit");
///     }
/// }
/// ```
pub struct GuestMode<'a>;

impl<'a> GuestMode<'a> {
    /// Creates a guest execution object.
    ///
    /// The `context` contains the vCPU state to execute. The `interrupt_port`
    /// and `timer_port` are kernel-provided policy objects consulted before VM
    /// entry. Creating this value does not enter the guest; use
    /// [`Self::execute`] to run the vCPU.
    pub fn new(
        context: &'a Mutex<GuestContext>,
        interrupt_port: &'a SpinLock<dyn GuestInterruptPort>,
        timer_port: &'a SpinLock<dyn GuestTimerPort>,
    ) -> Self;

    /// Runs the guest with the supplied guest physical memory space.
    ///
    /// Before VM entry, this method initializes or loads the VMCS, marks the
    /// vCPU as running, prepares guest interrupts and timers, and loads the
    /// current [`GuestContext`] state into hardware. After VM exit, it saves
    /// the hardware vCPU state back into `GuestContext` and restores the host
    /// CPU state before ordinary kernel execution resumes.
    ///
    /// Some VM exits are part of OSTD's low-level CPU contract and are handled
    /// internally. For example, OSTD handles CPUID, control-register access,
    /// MSR read/write, interrupt-window, and external-interrupt exits without
    /// returning them to the kernel. Exits that require higher-level policy or
    /// device emulation are returned as [`GuestExitInfo`].
    ///
    /// The `guest_mem` argument defines the guest physical address space for
    /// this run. This method obtains the EPT pointer from `guest_mem`
    /// internally, so safe kernel code cannot supply an arbitrary EPT root.
    ///
    /// If the vCPU is waiting for SIPI, this method returns a synthetic
    /// `HLT`-style exit without entering guest execution.
    pub fn execute(
        &mut self,
        guest_mem: &GuestPhysMemSpace,
    ) -> Result<GuestExitInfo>;
}

// ostd/src/vm/interrupt.rs

/// Provides guest interrupt injection policy to [`super::GuestMode`].
///
/// `GuestMode` uses this port before VM entry to choose whether an external
/// interrupt should be injected into the guest. If it commits an interrupt to
/// the VMCS injection fields, it calls
/// [`GuestInterruptPort::accept_interrupt`] so the kernel-side interrupt model
/// can synchronize its state.
///
/// The implementation is supplied by the kernel. It may model a virtual
/// interrupt controller, such as a local APIC, or it may be a policy object
/// that never offers interrupts.
pub trait GuestInterruptPort {
    /// Returns the next external interrupt vector to offer for injection.
    ///
    /// This method is a query. It should not consume the interrupt because
    /// `GuestMode` may find that the guest cannot accept it yet and enable
    /// interrupt-window exiting instead. Returning `None` means that no
    /// external interrupt should be offered for this VM entry.
    ///
    /// An implementation that does not inject guest interrupts can always
    /// return `None`.
    ///
    /// Implementations should return vectors suitable for external interrupt
    /// injection. On x86, vectors below 32 are reserved for exceptions and are
    /// ignored by `GuestMode` for external interrupt injection.
    fn check_pending_interrupt(&self) -> Option<u8>;

    /// Marks an interrupt vector as accepted for injection.
    ///
    /// `GuestMode` calls this method only after it has committed the vector to
    /// the VMCS injection fields for the next VM entry. Implementations should
    /// update their state accordingly. For a virtual APIC, this usually means
    /// moving the vector from a pending state to an in-service state and
    /// refreshing any priority bookkeeping.
    fn accept_interrupt(&mut self, vector: u8);
}

// ostd/src/vm/timer.rs

/// Provides guest timer interrupt policy to [`super::GuestMode`].
///
/// `GuestMode` uses this port before VM entry to ask when a VM exit should
/// happen so the kernel can publish a virtual timer interrupt in time.
pub trait GuestTimerPort {
    /// Returns the next guest timer deadline after `current_tsc`.
    ///
    /// The `current_tsc` argument is the current guest-visible TSC value. The
    /// returned deadline is also expressed in guest-visible TSC cycles.
    ///
    /// Returning `Some(deadline)` asks OSTD to arrange a VM exit when that
    /// deadline is reached. After that VM exit, `GuestMode` checks the paired
    /// [`super::GuestInterruptPort`] before the next VM entry, so the kernel
    /// implementation should publish any expired timer interrupt there.
    /// Returning `None` means that this timer port has no active deadline for
    /// the next guest run.
    ///
    /// If the timer has already expired at `current_tsc`, the implementation
    /// should update its timer state before returning. This usually means
    /// queuing a pending timer interrupt, advancing or clearing its internal
    /// next deadline, and returning the next active deadline if one remains.
    fn check_deadline(&mut self, current_tsc: u64) -> Option<u64>;
}

// ostd/src/arch/x86/vm/exit/mod.rs

/// Describes a VM exit that should be handled outside OSTD.
pub struct GuestExitInfo {
    /// The VMX exit reason.
    pub exit_reason: u32,
    /// The length of the instruction that caused the exit.
    pub instruction_len: u32,
    /// VMX exit qualification.
    pub exit_qualification: u64,
    /// Guest physical address associated with the exit, if any.
    pub guest_phys_addr: Gpaddr,
    /// Guest instruction pointer at the exit.
    pub guest_rip: Gpaddr,
}
````

**Soundness.**

`GuestMode::execute` wraps the VMX transition and the sensitive CPU-state
updates around it, including VMCS setup and guest CR2/CR3 state. After guest
state is loaded and before host state is restored, OSTD prevents the run from
migrating away from the current physical CPU or being interleaved with ordinary
kernel execution. The kernel receives only `GuestExitInfo`; it cannot directly
issue VMX instructions, edit host-only VMCS fields, or corrupt host state
through this API.

The interrupt and timer ports are safe policy traits. A buggy kernel-side
implementation may inject the wrong virtual interrupt or choose a poor timer
deadline, but this can only affect guest behavior or latency. It must not
corrupt host CPU state or host memory.

**Minimality.**

The type is deliberately small. It borrows a `GuestContext`, a guest interrupt
port, and a guest timer port from the kernel. The kernel decides what LAPIC or
timer model to provide, but OSTD decides how and when those policy decisions
are reflected in VMCS fields.

**Expressiveness.**

`GuestMode` performs VMX operations according to the contents of
`GuestContext`, so its expressiveness mostly follows from the expressiveness of
`GuestContext` described above. It adds two ports for interrupt and timer state,
which are intentionally kept out of `GuestContext`. Together, the context and
ports let the kernel run a guest while providing virtual interrupts and timer
events.


### Building a Hypervisor With the OSTD APIs

The kernel can build a complete hypervisor by composing the OSTD objects above
with kernel-owned policy and device state. A minimal Rust-like pseudo-code flow
looks like this:

```rust
fn create_and_run_guest(image: GuestImage, userspace_mem: UserRange) -> Result<()> {
    // Initialize the OSTD virtualization mechanism once during device setup.
    ostd::vm::init()?;

    // Create the VM and install userspace-backed guest RAM into the GPA space.
    let guest_mem = GuestPhysMemSpace::new();
    let frames: Vec<UFrame> = pin_userspace_frames(userspace_mem)?;
    guest_mem.set_memory_region(
        0,
        userspace_mem.start,
        image.ram_gpa,
        userspace_mem.len,
        frames,
        guest_mem_prop(),
    )?;

    // Create a vCPU context and let the kernel choose the guest boot state.
    let context = Mutex::new(GuestContext::new(0)?);
    context.lock().set_regs(image.boot_regs());
    context.lock().set_sregs(image.boot_sregs());

    // Kernel-owned policy ports provide virtual interrupts and timers.
    let interrupts = SpinLock::new(KernelInterruptPort::new());
    let timers = SpinLock::new(KernelTimerPort::new());
    let mut guest_mode = GuestMode::new(&context, &interrupts, &timers);

    loop {
        let exit = guest_mode.execute(&guest_mem)?;
        match decode_exit(exit) {
            Exit::Mmio(access) => emulate_mmio(access)?,
            Exit::Io(access) => forward_to_userspace_vmm(access)?,
            Exit::Hlt | Exit::Shutdown => return Ok(()),
            Exit::Other(exit) => reflect_exit_to_userspace(exit)?,
        }
    }
}
```

This example shows the intended split. OSTD owns the mechanisms needed to enter
and leave guest mode safely. The kernel owns VM lifecycle, memory-slot policy,
vCPU file semantics, interrupt-chip state, device emulation, and userspace
handoff.

### Minimal OSTD Extension Principle

The OSTD extension is intentionally limited to mechanisms that are sensitive
under the framekernel soundness model: VMX instructions, VMCS lifecycle, EPT
page tables, VM-entry/exit CPU-state transitions, and guest physical memory
mapping. KVM ABI details, APIC emulation policy, memory-slot policy, file
descriptor lifetimes, and userspace device models remain in safe `kernel/` or
userspace. This gives the kernel enough safe handles to build KVM efficiently
without moving a full VMM, a KVM ABI implementation, or container policy into
OSTD's memory-safety TCB.

## Drawbacks, Alternatives, and Unknowns

### Unresolved Questions

* When should VMXON and VMXOFF be executed? One option is for OSTD to expose
  safe wrappers for VMXON, VMXOFF, and their per-CPU backing frames, while the
  kernel decides when to enable or disable VMX operation. Another option is to
  bind VMXON/VMXOFF to the lifetime of `GuestContext` or `GuestMode`, so OSTD
  enables VMX before any vCPU can run and disables VMX after no vCPU can run.
* Is the current OSTD hypervisor interface stable enough for future
  acceleration features such as vAPIC acceleration? If those features require
  exposing additional VMCS controls, interrupt virtualization state, or APIC
  backing pages, the `GuestContext`, `GuestMode`, and `GuestPhysMemSpace`
  boundaries may need to evolve.

## Prior Art and References

* [Asterinas OSTD soundness analysis](../ostd/soundness/README.md)
* [Linux KVM API documentation](https://www.kernel.org/doc/html/latest/virt/kvm/api.html)
* [`kvm-hello-world`](https://github.com/dpw/kvm-hello-world)
* [`kvmtool`](https://github.com/kvmtool/kvmtool)
* [Intel 64 and IA-32 Architectures Software Developer Manuals](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html)
