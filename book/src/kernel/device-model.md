# The Device Model

Asterinas manages devices today, but it does not have a *device model*: a single
place that knows which devices exist, how they are attached to one another, which
driver serves each of them, and what user space is told about all of it. This
document proposes one, modeled on Linux's driver core but written in a Rust-native
way, and describes a prototype that implements it and converts the `mem` character
devices (`/dev/null` and its siblings) to run on it.

The document is self-contained. It assumes only that you know what a kernel
component is in Asterinas; the Background section teaches both the `systree` and
sysfs machinery the design builds on and the Linux mechanisms it copies. Claims
about Linux name the function they come from wherever there is a single one to
name, so that you can check them against the v7.2 source at
<https://elixir.bootlin.com/linux/v7.2/source>.
Code shown here is the prototype's own code rather than pseudocode: sometimes only
a signature, elided where a comment says so, and with a few doc comments expanded
for this document. The one exception is the Overview's worked example, which is
fictional, but written in the shapes real code takes.

The prototype is five commits on top of `ab9a4cfdc`, on the branch
[`device-model`](https://github.com/tatetian/asterinas/tree/device-model).
Every listing below can be read in place there, and the closing section lists the
commits one by one.

- [Motivation](#motivation)
- [Background](#background)
- [Requirements](#requirements)
- [Overview](#overview)
- [Design](#design)
- [Limitations, Discussions, and Future Work](#limitations-discussions-and-future-work)

# Motivation

## What Asterinas has today

Asterinas already boots with disks, terminals, input devices, a random-number
source and more. The bookkeeping behind them is spread across four unrelated
mechanisms:

- **Two number registries.** `kernel/core/src/device/registry/{char,block}.rs`
  map a device number to something openable. Registering in one of them also
  creates the `/dev` node, as a side effect of registration.
- **Per-driver lists.** virtio-blk keeps its own list of disks, the input core
  keeps its own list of input devices, and so on. Each list has its own locking,
  its own iteration order and its own idea of what "removed" means.
- **A sysfs tree with almost nothing in it.** The `systree` component holds an
  in-memory tree that sysfs renders, but only a handful of nodes are ever added:
  `/sys/kernel`, the cgroup and TSM nodes, and little else. `/sys/devices`,
  `/sys/bus`, `/sys/class` and `/sys/dev` do not exist at all — except under TDX,
  where the TSM measurement code fabricates the three directories it needs
  (`/sys/devices/virtual/misc`) with hand-rolled nodes of its own, which is the
  clearest symptom of the problem below.
- **Ad hoc naming.** Whether a device gets a name in `/dev`, and what it is called,
  is decided by whichever driver creates it.

Nothing ties these together. A device has no parent, no subsystem, and no driver
that anything outside its own driver can observe.

## Why that is a problem

**User space cannot find devices.** The interfaces Linux user space actually uses
are missing. `udevadm`, `systemd`, `lsblk`, `libinput` and the `libblkid`-based
tools all start from one of:

```text
/sys/class/block/vda          # every block device, by class
/sys/dev/char/1:3             # a device by its number
/sys/devices/.../vda/dev      # the number of a device found by walking the tree
/sys/block/vda/queue/...      # per-device tunables
```

None of these paths can exist without a device model, because each of them is an
*index into a device tree that does not exist yet*. Booting a distribution image
that runs `udevd` therefore either degrades to static `/dev` handling or fails.

**Every driver reinvents the same plumbing.** Without a shared registration path,
each driver that wants to appear in sysfs must create its own nodes, choose its own
attribute names, invent its own removal logic, and remember to create and delete
its `/dev` node. That is how the current TSM measurement node works, and it is
already inconsistent with everything around it.

**Nothing announces change.** Asterinas has the `NETLINK_KOBJECT_UEVENT` socket
family and the message types that go with it, including the parser for the
synthetic events user space writes, but no producer: no code ever emits an `add`
event, because no code owns the notion of "a device was added". Hotplug is
therefore invisible to user space even where the transport exists.

**Removal is undefined.** Because registration is a side effect scattered across
mechanisms, un-registration is too. Some drivers remove their `/dev` node, some do
not; nobody unbinds a driver, because binding is not modeled.

## What this design aims to do

Give Asterinas one device model that:

1. holds every device in one tree with a real parent relationship;
2. matches devices with drivers through buses, and presents them to user space
   through classes, so that a driver is written once and its sysfs surface follows;
3. produces the `/sys` layout Linux tools expect, including the three indexes and
   the symlinks between them;
4. owns `/dev` node creation and uevent emission, so that they happen exactly once
   per device, at the right point in registration;
5. does all of that with Rust's type system doing the work that Linux does by
   convention, so that the invalid states Linux merely avoids are states this model
   cannot express.

The prototype proves the design by converting a real subsystem end to end and by
exercising buses, drivers and classes in kernel-mode tests.

# Background

This section covers the Linux mechanisms the design copies, and the Asterinas
mechanisms it builds on. Skip either half if you already know it.

## Linux's driver core, in the parts that matter

Linux keeps one structure for every device it manages, `struct device`, defined in
`include/linux/device.h` and driven by `drivers/base/`. Five fields carry the
model:

| Field | Meaning |
|---|---|
| `parent` | the device this one is reached through; makes all devices one tree |
| `bus` | the bus it was enumerated on, if any |
| `class` | the class it belongs to, if any |
| `type` | a finer kind within the bus or class, if any |
| `devt` | its device number, if user space can open it |

### The tree and the three indexes

Every device gets a directory under `/sys/devices`, nested by `parent`. That is
the *topology*: it says how data physically reaches the device. On a QEMU q35
machine with a virtio disk, the topology reads (these are the real names from a
Linux v7.2 guest, where `virtio1` at function `0000:00:04.0` is the network card):

```text
/sys/devices/pci0000:00/0000:00:02.0/virtio0/block/vda
             ^          ^            ^       ^     ^
             |          |            |       |     the disk, a class device
             |          |            |       a glue directory named after the class
             |          |            the virtio device on the PCI function
             |          the PCI function (a bus device)
             the PCI host bridge (a device with neither bus nor class)
```

Because walking that tree is not how anyone finds a device, Linux adds three
indexes, all of them directories full of symlinks pointing back into the tree:

```text
/sys/bus/virtio/devices/virtio0 -> ../../../devices/pci0000:00/0000:00:02.0/virtio0
/sys/class/block/vda            -> ../../devices/pci0000:00/0000:00:02.0/virtio0/block/vda
/sys/dev/block/253:0            -> ../../devices/pci0000:00/0000:00:02.0/virtio0/block/vda
```

and, inside each device directory, back links to the indexes:

```text
vda/subsystem  -> ../../../../../../class/block               # which subsystem owns me
vda/device     -> ../../../virtio0                            # my parent, for a class device
virtio0/driver -> ../../../../bus/virtio/drivers/virtio_blk   # who drives me
```

### Bus versus class

A **bus** is how a device is *found and addressed*: PCI, virtio, platform. A bus
owns a matching rule and a list of drivers. A **class** is what a device *looks
like to user space*: `block`, `input`, `tty`, `mem`. A class owns the naming of
`/dev` nodes and the attributes every member shows.

A device belongs to at most one of them. This is worth dwelling on, because it is
the rule that shapes the whole model. A virtio disk is *two* devices in Linux: the
virtio device `virtio0` on the virtio bus, which a driver binds to, and the block
device `vda` in the `block` class, which the driver creates as the disk's
user-facing face and makes a child of `virtio0`. The second device is the *class
view* of the same hardware. Linux has no single name for the pattern, so this
document just calls `vda` a class device and `virtio0` its parent. The
alternative, one device that is both, would need two `subsystem` links in one
directory, two sets of attributes competing for the same names, and two answers to
"where does it live in the tree".

`get_device_parent` in `drivers/base/core.c` turns those rules into a directory:

- a class device whose parent is also a class device goes directly inside it
  (an NVMe namespace inside its controller, a partition inside its disk), unless
  its own class defines a sysfs namespace type, as `net` does, in which case it
  keeps a glue directory because that directory is what scopes the names;
- a class device under any other parent goes into a **glue directory** named after
  its class, created inside the parent and shared by later siblings (`block/` in
  the path above);
- a class device with no parent goes under `/sys/devices/virtual/<class>/`;
- a bus device or a device with neither goes directly under its parent, except
  that a parentless bus device may go under a root the bus itself supplies
  (`bus_get_dev_root`, which `cpu` uses); this design does not model that, and R3
  says so.

### Registration: `device_add`

One function creates every artifact. `device_add` (`drivers/base/core.c`) does, in
order: fix the name; choose the parent directory, creating `virtual` or a glue
directory if needed; create the directory; create the `uevent` file; create the
class symlinks; add the attribute groups of the class, the type and the device;
hand the device to its bus, which adds the bus's attributes, the
`/sys/bus/<bus>/devices/<name>` link and the `subsystem` link; add the power
management group, which is the `power/` directory (`dpm_sysfs_add`); add the `dev`
attribute, the `/sys/dev/{char,block}/M:m` link and a devtmpfs node if the device
has a number; send the `add` uevent; ask the bus to find a driver; and finally link
the device into its parent's child list and its class's member list, notifying any
class interfaces. `device_del` undoes roughly that list, but not in strict reverse:
it removes the `uevent` file before the other attributes, removes the attributes
before telling the bus, and emits `KOBJ_REMOVE` last, where `KOBJ_ADD` went out in
the middle.

### Binding

Binding is symmetric: when a device appears the bus offers it to every driver, and
when a driver registers the bus offers every unbound device to it. Both walks reach
`really_probe` (`drivers/base/dd.c`), which sets `dev->driver`, creates the two
`driver` links, calls the driver's `probe`, and *after* a successful probe adds the
driver's attribute groups. A failed probe removes the links and lets the next
driver try. User space can intervene through four files:

```text
/sys/bus/<bus>/drivers/<drv>/bind     # write a device name to bind it
/sys/bus/<bus>/drivers/<drv>/unbind   # write a device name to release it
/sys/bus/<bus>/drivers_autoprobe      # 0 disables automatic matching
/sys/bus/<bus>/drivers_probe          # write a device name to probe it once
```

### `/dev` and uevents

Two mechanisms carry the model to user space.

**devtmpfs** is a file system the kernel populates: when a device with a number is
registered, the kernel creates the node itself, so `/dev/null` exists before any
user-space daemon runs. The name and mode come from the device's name unless the
device type or the class overrides them through a `devnode` callback
(`device_get_devnode`), which is how `/dev/null` gets mode `0666` while `/dev/mem`
stays `0600`.

**uevents** are the kernel's announcements. Each carries an action, the device's
path, its subsystem, a global sequence number and a set of variables:

```text
ACTION=add
DEVPATH=/devices/virtual/mem/null
SUBSYSTEM=mem
MAJOR=1
MINOR=3
DEVNAME=null
DEVMODE=0666
SEQNUM=1234
```

They go out over a netlink multicast socket, and the same variables (minus the
first three and the last) are readable at any time from the device's `uevent`
attribute file. `udev` listens to the socket and applies policy: symlinks,
permissions, and running programs.

## What Asterinas provides to build on

### `systree`

`kernel/core/comps/systree` is an in-memory tree of named nodes. A node is a
**branch** (owns children), a **leaf** (attributes only) or a **symlink**. The
traits are `SysObj` (identity: id, name, parent, path), `SysNode` (attributes:
`node_attrs`, `read_attr_at`, `write_attr`), `SysBranchNode` (children: `child`,
`visit_children_with`) and `SysSymlink` (a target string). Helper structs
(`BranchNodeFields`, `NormalNodeFields`, ...) and `inherit_sys_*_node!` macros
supply the boilerplate. sysfs is a generic view over such a tree
(`kernel/core/src/fs/utils/systree_inode.rs`); cgroupfs and configfs are others.
A node's path is relative to the mount point, so the node this document calls
`/devices/virtual/mem/null` is what user space sees at
`/sys/devices/virtual/mem/null`, and it is the former that a uevent's `DEVPATH`
carries.

Two properties of `systree` as it stood matter here, and both had to change:

- **Attribute sets were frozen at construction.** A node declared its attributes
  when it was built and could never gain or lose one, so a driver could not add its
  files to a device that already existed.
- **A branch node's children were private to its type.** There was no operation by
  which another component could attach a node under someone else's node. That is
  right for cgroupfs, where the tree *is* the structure, and wrong for a device
  model, where a registration function must build a directory out of pieces owned
  by four different layers.

Symlinks store a literal string, so whoever creates one must compute the relative
target itself.

### Device numbers, devtmpfs and the component graph

The char and block registries map numbers to open functions. devtmpfs already
exists as a file system with a `create_node`/`delete_node` interface, populated
today by those registries. A device number is a `DeviceId` from the shared
`device-id` crate, a `MajorId` and a `MinorId` pair, and this design reuses it
rather than inventing another.

Two `systree` spellings appear throughout the code below: `SysStr`, which is an
owned string or a `&'static str` (a `Cow`), used for every node and attribute
name, and `SysPerms`, the permission bits of an attribute, with constants such as
`DEFAULT_RO_ATTR_PERMS` (0444), `DEFAULT_RW_ATTR_PERMS` (0644) and `OWNER_W`
(0200).

One structural constraint shapes the design: **a component under
`kernel/core/comps/` may not depend on the kernel crate `aster-core`.** The
dependency graph is acyclic and layered, and devtmpfs and the netlink socket live
in the kernel crate. A device model that lives in a component therefore cannot call
them directly. Components are initialized in stages (`Bootstrap`, `Kthread`,
`Process`) via `#[init_component]`.


# Requirements

The requirements below are what "implement Linux's device model" means, stated so
that the design can be checked against them. Each names the Linux function it is
derived from. They are grouped into what the model must *represent*, what it must
*publish*, and what the implementation must *respect*.

## Representation

- **R1. One device tree.** Every device has a name, a directory, and a parent
  unless it is a root. The tree is rooted at `/sys/devices`.
- **R2. One subsystem per device.** A device belongs to a bus, or to a class, or to
  neither, never to both: Linux keeps both fields on `struct device`, but
  `get_device_parent` places any device that has a class as a class device, and a
  device with both cannot be added at all, because the class and the bus each try
  to create the `subsystem` link and the second fails. A model that cannot express
  the combination therefore loses nothing.
- **R3. Placement rules.** A class device under a non-class parent gets a glue
  directory named after its class; one with no parent goes under
  `/sys/devices/virtual/<class>/`; bus and bare devices sit directly under their
  parent (`get_device_parent`). A parentless bus device may also go under a
  directory the bus provides, as `cpu` devices do (`bus_get_dev_root`).
- **R4. Glue directories are shared and transient.** Siblings of the same class
  share one; it disappears when its last device leaves
  (`class_dir_create_and_add`, `cleanup_glue_dir`).
- **R5. Device types.** A device may carry a finer kind within its subsystem, which
  contributes attributes, uevent variables and a `/dev` naming policy, and which
  user space reads as `DEVTYPE`.

## Publication

- **R6. Three indexes.** `/sys/bus/<bus>/devices/<name>`, `/sys/class/<class>/<name>`
  and `/sys/dev/{char,block}/<major>:<minor>` are symlinks to the device directory.
- **R7. Back links.** A device with a subsystem has a `subsystem` symlink; a bound
  bus device has a `driver` symlink that the driver directory answers with a link
  back by device name (`bus_add_device`, `device_add_class_symlinks`,
  `driver_sysfs_add`). A class device with a parent has a `device` symlink unless
  its device type opts out, as Linux's block partitions do.
- **R8. Attributes from several layers.** A device directory holds `uevent` always,
  `dev` when it has a device number, and the attribute tables of its subsystem, its
  device type, itself, and, once bound, its driver (`device_add_attrs` for the
  class, type and device tables, `bus_add_device` for a bus's, `really_probe` for
  the driver's).
- **R9. Device numbers and `/dev`.** A device with a number publishes `dev` as
  `major:minor`, gets its `/sys/dev` link, and asks devtmpfs for a node whose name
  and mode a `devnode` callback may override (`devtmpfs_create_node`,
  `device_get_devnode`).
- **R10. Uevents.** Every add, remove, bind, unbind and change produces an event
  carrying `ACTION`, `DEVPATH`, `SUBSYSTEM`, `SEQNUM`, the number, name and mode of
  the `/dev` node as `MAJOR`, `MINOR`, `DEVNAME`, `DEVMODE`, and `DEVTYPE` and
  `DRIVER`, to which the bus, class and type may add more. The `uevent` attribute
  shows the device variables and, when written, resends an event (`dev_uevent`,
  `uevent_store`).

## Behavior

- **R11. Buses match and bind.** A bus offers each new device to every driver and
  each new driver to every unbound device, binds the first driver whose `probe`
  succeeds, and unbinds on removal (`bus_probe_device`, `driver_attach`,
  `really_probe`, `device_release_driver`).
- **R12. User-space control of binding.** Under `/sys/bus/<bus>`, a device name
  written to `drivers/<drv>/bind` or `unbind` binds or unbinds it,
  `drivers_autoprobe` is a `0`/`1` switch, and a name written to `drivers_probe`
  probes one device.
- **R13. Class interfaces.** Code may ask to be told of every member of a class,
  including members that already exist (`class_interface_register`).
- **R14. Atomic registration.** A failed registration leaves nothing behind;
  removal undoes the steps in reverse order.
- **R15. Hot removal is visible.** sysfs must stop showing a removed device even
  when its directory was looked up before, which means the VFS dentry cache has to
  be revalidated.

## Implementation constraints

- **R16. Component boundaries.** The model lives in a component that cannot depend
  on the kernel crate, yet devtmpfs and the uevent socket live there.
- **R17. Make invalid states unrepresentable.** Where Linux relies on convention
  (a device with one subsystem, a driver bound to devices of its own bus, an
  attribute attached to the kind of device it was written for), the Rust design
  should rely on types, so that the mistake is a compile error rather than a
  runtime surprise.
- **R18. Do not fork `systree`.** Whatever the model needs from the tree must be a
  change `systree`'s other users (sysfs, cgroupfs, configfs) can live with.

## Non-goals

Power management, device links between consumers and suppliers, deferred probing,
module autoloading, and namespaces are out of scope for this design. They are
Linux features that sit on top of the model rather than inside it, and none of them
is needed by the devices Asterinas has today. Out of scope here means deferred, not
rejected: the future work at the end of this document sketches where power
management, `driver_override` and deferred probing would attach.

# Overview

## The vocabulary, as Rust items

The Background said what buses, classes, devices and drivers *are*. This section
says what they are *here*: six items an author writes or names, and the calls that
put them to work. The minimal example below is one of each, so it is worth
reading this list as a preview of that code.

**A bus is a type implementing `Bus`.** It fixes the directory name under
`/sys/bus`, the payload every device on it carries (`Bus::Device`), the match data
a driver advertises (`Bus::MatchData`), and the rule that compares the two.

**A class is a type implementing `Class`.** It fixes the directory name under
`/sys/class`, the payload its devices carry, the attributes every member gets, and
the name and mode of the `/dev` node, if there is one.

**A device is one of three structs**, and which one it is says which subsystem
owns it: `BusDevice<B>` for a device enumerated on bus `B`, `ClassDevice<C>` for a
device presented through class `C`, and `BareDevice` for one that is neither and
exists to be a parent, like a PCI host bridge. The two that have a subsystem
dereference to its payload, so a driver reads `dev.vendor` rather than casting.

**A driver is a type implementing `Driver<B>`.** The `B` is what ties it to one
bus. Its `probe` takes an `&Arc<BusDevice<B>>` and either takes the device over or
declines it.

**A device type is a `DeviceType<D>` static**, naming a finer kind within a bus or
class — `disk` against `partition` — and carrying the attributes, uevent variables
and `/dev` policy that go with that kind.

**An attribute is an `Attr<D>`**: a file name, permissions, and up to two function
pointers, declared against the device struct it belongs to. A device's files come
from five layers that the model merges in a fixed order: the model itself, the
subsystem, the device type, the device, and the bound driver.

**Registration is one function.** `add` takes a built device and produces
everything user space sees; `remove` takes it all away again. Registering a bus, a
class or a driver is likewise one call each.

## What you write, and what the model does

The division of labor is the point of the design, so it is worth being exact about
it before seeing it in code.

You write the trait implementations above, the payload structs behind them, and the
`probe` that turns a matched device into whatever user space should see. That is
all. In particular you never implement a `SysTree` node, never create a directory,
never compute a symlink target, and never touch `/dev`.

The model does the rest, the same way for every device:

- chooses the directory's place in `/sys/devices`, creating the glue directory or
  the `virtual` directory when the rules call for one — the sysfs tree is a *view*
  derived from the parent relation, not that relation itself, which is why it holds
  directories that are not devices;
- creates the attribute files of all five layers;
- creates the `subsystem`, `device` and `driver` symlinks, and the entries in
  `/sys/bus/<b>/devices`, `/sys/class/<c>` and `/sys/dev/{char,block}`, computing
  every relative target;
- creates the `/dev` node and announces the device;
- offers the device to the bus's drivers, and adds the winner's attributes;
- and undoes all of it, in reverse, on removal.

One structural decision shapes how that is possible, and it is the only piece of
Rust design worth stating before the example: **the type parameters sit on the
relationships, not on the tree.**

A driver is `Driver<B>`, an attribute is `Attr<D>`, a device in a class is
`ClassDevice<C>`. Those parameters are what the compiler checks: a driver can only
be registered on the bus it was written against, a class's attribute can only be
attached to that class's devices, and a device's subsystem is a property of its
Rust type rather than a field that could hold the wrong thing.

The tree itself carries no parameter. A PCI function's children can be a virtio
device, a glue directory and a class device at once, and a device's parent may be a
directory that is not a device at all, so parents, children and the registration
sequence all work on one trait object, `dyn AnyDevice`, over one shared struct,
`DeviceBase`. That is what lets `add` exist once rather than once per bus.

## A minimal example

The whole model fits in one page of code. Here is a complete, if fictional,
subsystem: a `toy` bus whose devices carry a vendor and a model number, a `toyblk`
class for the disks such devices turn out to be, and a driver that binds the first
and creates the second. This is the shape every real conversion will take.

The snippets below elide `use` lines and error handling that add nothing; every
other line is what the code looks like. The names they use come from
`aster_device` (`Bus`, `Class`, `Driver`, `Attr`, `BusDevice`, `ClassDevice`,
`BareDevice`, `BusHandle`, `ClassHandle`, `DevNum`, `DevNode`, `DeviceType`,
`UeventVars`, `Result`) and from the `device-id` crate (`DeviceId`, `MajorId`,
`MinorId`).

**Define the bus.** The bus fixes what its devices carry (`Device`), what a driver
declares to select them (`MatchData`), and the matching rule:

```rust
struct ToyBus;

/// What every device on the toy bus carries.
struct ToyDev {
    vendor: u32,
    model: u32,
}

/// What a toy driver declares.
struct ToyMatch {
    vendor: u32,
}

/// Two attributes every device on this bus shows.
const TOY_DEV_ATTRS: &[Attr<BusDevice<ToyBus>>] = &[
    Attr::ro("vendor", |dev, w| { writeln!(w, "{:#06x}", dev.vendor)?; Ok(()) }),
    Attr::ro("model", |dev, w| { writeln!(w, "{:#06x}", dev.model)?; Ok(()) }),
];

impl Bus for ToyBus {
    const NAME: &'static str = "toy";
    type Device = ToyDev;
    type MatchData = ToyMatch;

    fn matches(&self, dev: &ToyDev, data: &ToyMatch) -> bool {
        dev.vendor == data.vendor
    }

    fn dev_attrs(&self) -> &'static [Attr<BusDevice<Self>>] {
        TOY_DEV_ATTRS
    }

    fn uevent(&self, dev: &BusDevice<Self>, vars: &mut UeventVars) {
        vars.add("MODALIAS", format_args!("toy:v{:08X}m{:08X}", dev.vendor, dev.model));
    }
}
```

Note `dev.vendor` inside the attribute callback: the callback receives
`&BusDevice<ToyBus>`, which dereferences to `ToyDev`, so the bus's own fields are
reachable without a cast.

**Define the class** the driver will expose its disks through:

```rust
struct ToyBlock;

struct ToyDisk {
    sectors: u64,
}

const TOY_BLOCK_ATTRS: &[Attr<ClassDevice<ToyBlock>>] =
    &[Attr::ro("size", |dev, w| { writeln!(w, "{}", dev.sectors)?; Ok(()) })];

impl Class for ToyBlock {
    const NAME: &'static str = "toyblk";
    type Device = ToyDisk;

    fn dev_attrs(&self) -> &'static [Attr<ClassDevice<Self>>] {
        TOY_BLOCK_ATTRS
    }

    fn devnode(&self, _dev: &ClassDevice<Self>) -> Option<DevNode> {
        // Nodes of this class are group-readable and -writable.
        // `path: None` keeps the device's own name.
        Some(DevNode { path: None, mode: Some(0o660) })
    }
}
```

**Define the driver.** Its `probe` is where a real driver would talk to the
hardware; here it just creates the class device that user space will open. The
driver keeps an `Arc<ClassHandle<ToyBlock>>`: the handle `register_class` returns,
which is the class's runtime state. `dev.base()`, in its `remove`, reaches the part
of a device that every kind shares; the Design section covers both.

```rust
struct ToyDiskDriver {
    match_data: ToyMatch,
    class: Arc<ClassHandle<ToyBlock>>,
}

const BOUND_BY: &[Attr<BusDevice<ToyBus>>] =
    &[Attr::ro("bound_by", |_dev, w| { writeln!(w, "toy_disk")?; Ok(()) })];

impl Driver<ToyBus> for ToyDiskDriver {
    fn name(&self) -> &str { "toy_disk" }

    fn match_data(&self) -> &ToyMatch { &self.match_data }

    /// Attributes the device gains while this driver is bound to it.
    fn dev_attrs(&self) -> &'static [Attr<BusDevice<ToyBus>>] { BOUND_BY }

    fn probe(&self, dev: &Arc<BusDevice<ToyBus>>) -> Result<()> {
        let disk = ClassDevice::builder(&self.class, format!("td{}", dev.model), ToyDisk { sectors: 8 })
            .parent(dev.clone())
            .devnum(DevNum::block(DeviceId::new(MajorId::new(200), MinorId::new(dev.model))))
            .build();
        aster_device::add(&disk)
    }

    fn remove(&self, dev: &Arc<BusDevice<ToyBus>>) {
        for child in dev.base().child_devices() {
            let _ = aster_device::remove(&child);
        }
    }
}
```

**Name the device type.** A device type is a `static` that a device points at; it
names the kind for user space and can carry attributes and policies of its own. A
real `block` class would put its types on the class device, to tell a whole disk
from a partition; this example puts one on the bus device instead, only to show
that a bus device may have a type too:

```rust
static DISK_TYPE: DeviceType<BusDevice<ToyBus>> = DeviceType::named("disk");
```

**Wire it up.** Registering the bus and the class creates their directories;
registering the driver offers it every unbound device; adding a device offers it to
every driver:

```rust
let bus = aster_device::register_bus(ToyBus)?;   // an `Arc<BusHandle<ToyBus>>`
let class = aster_device::register_class(ToyBlock)?;
bus.register_driver(Arc::new(ToyDiskDriver { match_data: ToyMatch { vendor: 1 }, class }))?;

// A bare device to be the root of this fictional machine.
let root = BareDevice::new_root("toy0000:00");
aster_device::add(&root)?;

// A device on the bus. `add` places it, publishes it, and probes for a driver.
let dev = BusDevice::builder(&bus, "0000:00:01.0", ToyDev { vendor: 1, model: 3 })
    .parent(root.clone())
    .dev_type(&DISK_TYPE)          // `static DISK_TYPE: DeviceType<BusDevice<ToyBus>>`
    .build();
aster_device::add(&dev)?;
```

**What user space sees.** The example above produces the whole sysfs surface, with
no further calls: the bus device with its attributes and its `driver` link, the
class device the driver created inside a glue directory, all three indexes, and a
`/dev` node. In this and every later listing, a trailing `/` marks a directory,
`->` gives a symlink's target, and `# =` gives a file's contents, with the lines of
a multi-line file such as `uevent` separated by spaces. Below the blank
line the listing switches to the three indexes and the bus's own control files, and
there it names only the entries: the directories holding them — `/sys/bus/toy/` and
its `devices/` and `drivers/toy_disk/`, `/sys/class/toyblk/`, `/sys/dev/block/` —
exist too.

```text
/sys/devices/toy0000:00/                                  # the bare root
/sys/devices/toy0000:00/uevent                            # = (empty: no subsystem, so no variables)
/sys/devices/toy0000:00/0000:00:01.0/                     # the bus device
/sys/devices/toy0000:00/0000:00:01.0/uevent               # = DEVTYPE=disk DRIVER=toy_disk MODALIAS=toy:v00000001m00000003
/sys/devices/toy0000:00/0000:00:01.0/vendor               # = 0x0001      (from the bus)
/sys/devices/toy0000:00/0000:00:01.0/model                # = 0x0003      (from the bus)
/sys/devices/toy0000:00/0000:00:01.0/bound_by             # = toy_disk    (from the driver)
/sys/devices/toy0000:00/0000:00:01.0/subsystem  -> ../../../bus/toy
/sys/devices/toy0000:00/0000:00:01.0/driver     -> ../../../bus/toy/drivers/toy_disk
/sys/devices/toy0000:00/0000:00:01.0/toyblk/              # glue directory, named after the class
/sys/devices/toy0000:00/0000:00:01.0/toyblk/td3/          # the class device
/sys/devices/toy0000:00/0000:00:01.0/toyblk/td3/dev       # = 200:3
/sys/devices/toy0000:00/0000:00:01.0/toyblk/td3/size      # = 8           (from the class)
/sys/devices/toy0000:00/0000:00:01.0/toyblk/td3/uevent    # = MAJOR=200 MINOR=3 DEVNAME=td3 DEVMODE=0660
/sys/devices/toy0000:00/0000:00:01.0/toyblk/td3/device    -> ../../../0000:00:01.0
/sys/devices/toy0000:00/0000:00:01.0/toyblk/td3/subsystem -> ../../../../../class/toyblk

/sys/bus/toy/drivers_autoprobe                            # = 1
/sys/bus/toy/drivers_probe                                # write-only
/sys/bus/toy/devices/0000:00:01.0             -> ../../../devices/toy0000:00/0000:00:01.0
/sys/bus/toy/drivers/toy_disk/bind                        # write-only
/sys/bus/toy/drivers/toy_disk/unbind                      # write-only
/sys/bus/toy/drivers/toy_disk/0000:00:01.0    -> ../../../../devices/toy0000:00/0000:00:01.0
/sys/class/toyblk/td3                         -> ../../devices/toy0000:00/0000:00:01.0/toyblk/td3
/sys/dev/block/200:3                          -> ../../devices/toy0000:00/0000:00:01.0/toyblk/td3

/dev/td3                                                  # brw-rw---- 200,3
```

Reading that listing against the code is the fastest way to understand the model:

- the **glue directory** `toyblk/` appeared because a class device was placed under
  a non-class parent (R3);
- the **`device` link** points at the parent, and its `../../..` depth is what tells
  you the class device sits one level deeper than its parent, inside the glue;
- the three **indexes** all point back at the same directory, which is why user
  space can start from a class, a number, or a bus;
- the **attributes** come from three different declarations, merged into one
  directory: the bus's `vendor` and `model`, the driver's `bound_by`, the class's
  `size`;
- the **`/dev` node** and the `DEVMODE` in the `uevent` file agree because both come
  from the same `devnode` callback, evaluated once.

## Why this design

The example above is the whole argument in miniature, so it is worth naming what it
shows before moving on to the implementation.

**A subsystem costs almost nothing to add.** The `toy` bus, the `toyblk` class, the
driver and the device fit in a page, and there is no boilerplate hiding elsewhere:
no `SysTree` node implementation, no directory creation, no symlink arithmetic, no
`/dev` bookkeeping, and no sysfs or `/dev` teardown to write: a driver's `remove`
disposes of the devices it created, and nothing else. A real conversion is the same
shape, which is what makes converting the rest of Asterinas's devices tractable
rather than a rewrite each time.

**The conventions are enforced, not documented.** Everything under `/sys/devices`
is produced by one function, so every device gets the same artifacts in the same
order. A driver cannot forget the `dev` attribute, place a class device in the
wrong directory, emit the `add` event before the device is complete, or leave an
index entry behind on removal, because it does not write any of that code. The
rules that Linux spreads across `device_add`, `get_device_parent`,
`device_add_class_symlinks` and `devtmpfs_create_node` live in one place here, and
a fix to them is a fix everywhere.

**The mistakes that matter are unrepresentable, not merely rejected.** A device in
both a bus and a class, a driver registered on a bus it was not written for, a
class's attribute attached to a bus device, a `probe` that receives the wrong
device type: none of these is a runtime check that might be missing, and none of
them compiles. The tree cannot be edited from outside the crate either, because the
trait that edits it is crate-private: its methods are on no trait a caller can name,
and the device structs deliberately do not implement it. That is the difference
between a model that documents its invariants and one that holds them.

**Types carry the relationships, so the code reads like the domain.** `Driver<B>`
says a driver belongs to a bus; `ClassDevice<C>` says a device is presented through
a class; `Attr<D>` says an attribute belongs to a kind of device. Because a bus or
class device dereferences to its subsystem's payload, a driver writes `dev.vendor`
where Linux writes a container-of macro and a cast.

**One implementation, checked once against the real thing.** Since the tree is
erased, placement, link computation, announcement and teardown exist in a single
copy. That copy is what produced `/dev/null`'s sysfs directory in the prototype,
and its output is identical, line for line, to a Linux v7.2 guest on the same
machine, apart from the `power/` directory this design does not model. The same
code path will produce `vda`'s.

**Removal is the exact inverse of registration.** `remove` undoes the eight steps
in reverse and refuses a device that still has children rather than orphaning them,
so a device is either fully present or fully gone. Today, un-registration in
Asterinas is as scattered as registration.

**It is testable without hardware.** A synthetic bus in the kernel-mode tests
exercises matching, probing, binding, unbinding, removal, the control files and the
glue-directory lifecycle, asserting paths and file contents rather than return
codes.

Everything after this point is how that is implemented.

# Design

The Overview above was the top-down pass: concepts first, then one example that
shows them working together. This section is the bottom-up one. It describes the
design a module at a time, from the pieces the tree needs up to the registration
sequence that uses all of them, and ends with the real conversion of `/dev/null`.
Most subsections read like the documentation of the module they describe: what the
module is for, what its types guarantee, and the code that matters. A few cut
across the modules instead — where the code lives, the changes to `systree` and
sysfs, the concurrency rules, the conversion checklist, the requirement check, and
the closing example. A reader who
wants the sequence first can read "Registration and removal" before the pieces; it
is written to be readable that way.

## Where the code lives

The model is a new component, `aster-device`, at
[`kernel/core/comps/device`](https://github.com/tatetian/asterinas/tree/device-model/kernel/core/comps/device).
It sits between the drivers, which are components above it — today only the kernel
crate, since no driver component has been converted yet — and `systree`, which is a
component beside it, and it owns every node under `/sys/devices`, `/sys/bus`,
`/sys/class` and `/sys/dev`.

```text
kernel/core/comps/device/src/
├── lib.rs            the component, the four root directories, the public API
├── device/
│   ├── mod.rs        DeviceBase, AnyDevice, Subsystem, DeviceType, the node macro
│   ├── registration.rs   add, remove, place, teardown, the builder
│   ├── bus_device.rs     BusDevice<B>
│   ├── class_device.rs   ClassDevice<C>
│   └── bare_device.rs    BareDevice
├── bus.rs            Bus, Driver, BusHandle<B>, DriverHandle<B>, binding
├── class.rs          Class, ClassInterface, ClassHandle<C>
├── attr.rs           Attr<D>, TyErasedAttr, AttrTable
├── error.rs          Error and Result, and the conversions to and from systree
├── node.rs           Dir, SymlinkNode, GlueDirs, Container, SysTreeEdit
├── devnum.rs         DevKind, DevNum, DevNodeRequest
├── uevent.rs         UeventAction, UeventVars, Uevent
├── hooks.rs          KernelHooks, HookSlot, install_hooks
└── test.rs           kernel-mode tests with a synthetic bus and class
```

The component is initialized at the `Bootstrap` stage, where it creates its four
root directories and attaches them to the primary `systree`:

```rust
#[init_component]
fn init() -> core::result::Result<(), ComponentInitError> {
    REGISTRY.call_once(|| Registry::new().expect("cannot create the device model roots"));
    Ok(())
}
```

`Registry` is the crate's one global. It holds the four root directories — keeping
the two `/sys/dev` subdirectories rather than their parent, which is attached and
then forgotten — the `virtual` directory, the glue directories under it, and the
registered subsystems, which it keeps alive for the life of the kernel, in the way
Linux's `bus_kset` and `class_kset` hold registered subsystems (though Linux,
unlike this design, can also unregister them):

```rust
struct Registry {
    devices: Arc<Dir>,
    virtual_dir: Arc<Dir>,
    bus: Arc<Dir>,
    class: Arc<Dir>,
    dev_char: Arc<Dir>,
    dev_block: Arc<Dir>,
    virtual_glue_dirs: GlueDirs,
    subsystems: Mutex<Vec<Arc<dyn SubsystemOps>>>,
}
```

Read those fields as a second table of contents: `Dir` is the plain directory type,
`GlueDirs` the per-parent set of class directories, and `SubsystemOps` the erased
view of a bus or class handle. Each gets a subsection of its own below, under *The
node types the model owns*, *Placement*, and *The one erased subsystem trait*.

`registry()` is the accessor for that global, and the code in the rest of this
document reaches the tree through it: `devices_root`, `bus_root`, `class_root` and
`dev_index(kind)` for the roots, `keep_subsystem` to hold a registered bus or class
for the life of the kernel, and `attach_into_virtual_glue_dir` /
`drop_virtual_glue_dir_if_empty` for the per-class directories under
`/sys/devices/virtual`.

### Errors

The crate has one error enum and one `Result` alias. Its variants are the refusals
the model can make, which is also a compact summary of what it checks:

```rust
/// Errors reported by the device model.
pub enum Error {
    /// The device has already been added, or is being added.
    AlreadyAdded,
    /// The device has not been added, or has already been removed.
    NotAdded,
    /// The parent device is not currently added.
    ParentNotAdded,
    /// The device still has child devices.
    HasChildren,
    /// A sibling with the same name already exists.
    NameConflict,
    /// The named object does not exist.
    NotFound,
    /// The name is empty or contains `/` or `NUL`.
    InvalidName,
    /// No driver accepted the device.
    NoDriver,
    /// The device is already bound to a driver.
    AlreadyBound,
    /// The device is not bound to a driver.
    NotBound,
    /// The driver rejected the device in `probe`.
    ProbeFailed,
    /// The driver has been unregistered.
    DriverUnregistered,
    /// An attribute callback failed.
    Attribute,
    /// The device has no device number, so it has no `dev` attribute.
    NoDevNum,
    /// Formatting an attribute value failed.
    Format,
    /// The value written to an attribute is invalid.
    InvalidValue,
    /// A resource (such as attribute IDs) is exhausted.
    ResourceUnavailable,
    /// The kernel hook failed to create or delete a device node.
    Hook,
    /// An error from the underlying `SysTree`.
    SysTree(aster_systree::Error),
}

pub type Result<T, E = Error> = core::result::Result<T, E>;
```

Two conversions matter. `From<aster_systree::Error>` folds the tree's errors into
the model's, so that a name collision detected by the tree arrives as
`NameConflict`. `From<Error> for aster_systree::Error` goes the other way, because
an attribute callback runs underneath sysfs and must answer in the tree's
vocabulary; the kernel crate maps the model's errors onto errnos in one place, so
`NotAdded` becomes `ENODEV`, `HasChildren` becomes `ENOTEMPTY`, and so on.

### The two structures, precisely

The *topology* is `DeviceBase::parent`, an `Option<Arc<dyn AnyDevice>>`, plus the
weak child list that mirrors it. The *view* is the `systree` tree. They are related
by exactly one function, `place`, and by the glue directories it creates. Nothing
else in the crate decides where a node goes, which is what keeps the derivation
rules (R3, R4) in one place instead of scattered across every driver.

## Changes to `systree` and sysfs

Three changes were needed to host the model — two in `systree`, one in the sysfs
inode layer — and a fourth that looked necessary was not. Only one of them is
visible to `systree`'s other users, and then only as a return type.

### A node may replace its attribute set, but no one may edit one

A driver's attributes appear when it binds and disappear when it unbinds, so the
set of files in a device's directory has to change after the directory exists.

The obvious way to allow that is to give `SysAttrSet` interior mutability and
public `add` and `remove` methods. That is the wrong shape, and it is worth saying
why, because the reason generalizes. `SysNode::node_attrs` is how a *consumer* —
sysfs, cgroupfs, configfs — reads a node's attributes. Interior mutability would
put `add` and `remove` on the far side of that call, so any consumer holding a
`&dyn SysNode` could invent attributes on a node it does not own. A file system
that renders the tree has no business editing it, and nothing in the type system
would have said so.

So `SysAttrSet` stays what it always was: an immutable value, built once by
`SysAttrSetBuilder`, with no way to change it afterwards. What changes is which set
a node points at:

```rust
/// Returns the attribute set of a `SysNode`.
///
/// The set is a snapshot: it is immutable, and a node whose attributes come
/// and go publishes a new set rather than editing this one, so a caller may
/// hold on to what it gets.
fn node_attrs(&self) -> Arc<SysAttrSet>;
```

Publishing is the node's own business, reachable only through the node's concrete
type, so a consumer that has only a trait object cannot do it. For a device that
type is `AttrTable`, which is crate-private to `aster-device`; every other node in
the tree is built with its set and never changes it.

The one thing this needs from `systree` is a way to derive the next set from the
current one without disturbing the attributes that survive, because sysfs derives
an inode number from the pair `(node id, attribute id)`: if binding a driver
renumbered the attributes beside it, every file in the directory would become a new
inode. So the builder can be seeded from an existing set, and it owns the id space:

```rust
impl SysAttrSetBuilder {
    /// Creates a builder holding the attributes of `set`, with their IDs.
    pub fn from_set(set: &SysAttrSet) -> Self;

    /// Adds an attribute, taking the lowest ID not already in use.
    pub fn add(&mut self, name: SysStr, perms: SysPerms) -> &mut Self;

    /// Removes an attribute by name, freeing its ID.
    pub fn remove(&mut self, name: &str) -> &mut Self;

    pub fn build(self) -> Result<SysAttrSet>;
}
```

An attribute that is present before and after keeps its id, so its file keeps its
inode; an id freed by `remove` can be taken later, which is what keeps the 8-bit id
space — 256 attributes per node, unchanged — from leaking across repeated binds.

Three things fall out of this that the mutable version did not give:

- **A reader sees a whole set.** `node_attrs` hands back a snapshot, so a directory
  listing cannot observe a set halfway through a driver binding.
- **An empty set can be shared.** Because a set is immutable, a node with no
  attributes can point at one static empty set, and the attribute-less branch nodes
  do, as they did before.
- **A device's files and their callbacks cannot disagree.** `AttrTable` keeps the
  published set and the callbacks under one lock and swaps them together, where
  before they were two locks that had to be kept in step by hand.

The cost is one `Arc` clone per lookup and one rebuild of a small map per bind. The
signature change reaches no file system: sysfs, cgroupfs and configfs all read
attributes through one shared inode layer, and it absorbed the change in four lines.

### Relative symlink targets are computed by the tree

sysfs symlinks are relative, and getting them right is fiddly enough that it should
be done once:

```rust
/// Computes the relative path from the directory `from_dir` to the target `to`.
///
/// Both arguments are absolute paths within one `SysTree`, as returned by
/// `SysObj::path`, where the root is `/`.
///
/// The result always ends with the last component of `to`, as the symlinks in
/// Linux's sysfs do: the relative path from `/devices/a/b/c` to `/devices/a`
/// is `../../../a`, not `../..`.
pub fn relative_path(from_dir: &str, to: &str) -> String {
    let from: Vec<&str> = from_dir.split('/').filter(|s| !s.is_empty()).collect();
    let to: Vec<&str> = to.split('/').filter(|s| !s.is_empty()).collect();
    let Some((last, to_parent)) = to.split_last() else {
        return String::from("/");
    };
    let common = from.iter().zip(to_parent.iter()).take_while(|(a, b)| a == b).count();

    let mut result = String::new();
    for _ in common..from.len() {
        result.push_str("../");
    }
    for component in &to_parent[common..] {
        result.push_str(component);
        result.push('/');
    }
    result.push_str(last);
    result
}
```

The rule in the doc comment is not obvious and is worth stating: the common prefix
is computed against the target's *parent*, so a link that points at one of its own
ancestors still names it. This is what Linux's `kernfs_get_target_path` does, and
it is why `td3/device` above reads `../../../0000:00:01.0` rather than `../../..`.

### sysfs revalidates cached names

Devices come and go, and the VFS caches directory entries. Without help, a removed
device keeps showing up and a newly added one stays invisible where a lookup
already failed. sysfs inodes now implement the VFS revalidation hooks: before the
VFS trusts a cached child it calls `revalidate_exists`, and drops the cached entry
if that returns `false`; `revalidate_absent` answers whether a cached *negative*
lookup may still be trusted, and this file system never trusts one.

```rust
fn revalidation_policy(&self) -> RevalidationPolicy {
    // Devices come and go, so the names under a directory must be checked
    // against the live `SysTree` rather than trusted from the dentry cache.
    match self.node_kind() {
        SysTreeNodeKind::Branch(_) | SysTreeNodeKind::Leaf(_) => {
            RevalidationPolicy::REVALIDATE_EXISTS | RevalidationPolicy::REVALIDATE_ABSENT
        }
        SysTreeNodeKind::Attr(..) | SysTreeNodeKind::Symlink(_) => RevalidationPolicy::empty(),
    }
}

fn revalidate_exists(&self, name: &str, child: &dyn Inode) -> bool {
    let Some(child) = child.downcast_ref::<Self>() else {
        return false;
    };

    let child_node_id = match child.node_kind() {
        // An attribute is still valid if its node still lists it with the
        // same id; an attribute removed and re-added is a new file.
        SysTreeNodeKind::Attr(attr, node) => {
            return node
                .node_attrs()
                .get(attr.name())
                .is_some_and(|current| current.id() == attr.id())
                && !node.is_attr_absent(name);
        }
        SysTreeNodeKind::Branch(node) => *node.id(),
        SysTreeNodeKind::Leaf(node) => *node.id(),
        SysTreeNodeKind::Symlink(node) => *node.id(),
    };

    // A child directory or symlink is still valid if the same node is
    // still attached under the same name.
    match self.node_kind() {
        SysTreeNodeKind::Branch(branch) => branch
            .child(name)
            .is_some_and(|current| *current.id() == child_node_id),
        _ => false,
    }
}

fn revalidate_absent(&self, _name: &str) -> bool {
    false
}
```

Comparing node ids rather than names is what makes a remove-then-add of the same
name produce a new inode rather than a resurrected one. cgroupfs already did this;
sysfs now does the same.

For an attribute the id comparison is the whole mechanism, and it is how a driver's
attributes disappear at unbind (R15): unbinding removes the entry from the node's
`SysAttrSet`, so the lookup returns `None` and the cached file is dropped. The
`is_attr_absent` conjunct beside it comes from the shared `systree` contract, where
cgroupfs uses it for attributes that are declared but conditionally hidden; every
node the device model owns answers `false` to it.

### What was not changed

`SysBranchNode` gained no `add_child` method, and the component does not edit
anyone else's nodes. Instead it defines its own node types and keeps the editing
operations in a trait that never leaves the crate:

```rust
mod private {
    /// The seal of [`super::Container`].
    pub trait Sealed {}
}

/// A node that holds children in the sysfs tree: a directory or a device.
///
/// The trait is sealed: only this crate implements it, and therefore only
/// this crate implements [`AnyDevice`](crate::AnyDevice). The operations that
/// edit a container's children are not on this trait; they are on the
/// crate-private [`SysTreeEdit`], which no trait object reachable from outside
/// the crate provides.
pub trait Container: SysBranchNode + Sealed {}

/// The crate-private editing view of a container.
///
/// Implemented by [`Dir`] and by `DeviceBase`, never by a device struct, so
/// that a `dyn AnyDevice` or a `dyn Container` cannot reach these operations.
pub(crate) trait SysTreeEdit: Send + Sync {
    fn attach_child(&self, child: Arc<dyn SysObj>) -> Result<()>;
    fn detach_child(&self, name: &str) -> Result<Arc<dyn SysObj>>;
    fn has_children(&self) -> bool;
    fn tree_path(&self) -> String;
}
```

`tree_path` is on `SysTreeEdit` rather than beside `SysObj::path`, which computes the
same string, only because a caller that is editing the tree already holds a
`&dyn SysTreeEdit` and should not need a second trait object to ask where it is.

Two details make the seal real rather than nominal. First, `SysTreeEdit` is
implemented by `Dir` and by `DeviceBase` — *not* by `BusDevice<B>` — so even a
`dyn AnyDevice` cannot reach `attach_child`: the methods are not on any trait the
device implements. Second, `systree` exposes two public mutators on a branch node,
`SysBranchNode::create_child` and `SysBranchNode::remove_child`, and the device
structs leave both at their refusing defaults, so both are closed off for
devices.

One `systree` method remains public on a device and is not sealed: `init_parent`,
which is how the tree sets a parent. It is not reachable by accident, and closing
it would mean changing `systree`'s own trait. The attribute set that `node_attrs`
returns needs no seal, because it is an immutable snapshot: a caller can read it
and keep it, and can do nothing else with it.

## The node types the model owns

Before the devices themselves, three small node types that the rest of the design
uses constantly.

**`Dir`** is a plain directory: the four roots, `virtual`, the index directories,
the glue directories, and the per-bus and per-driver directories. It is
crate-private, so the only ways to create one are registering a device, a bus, a
class or a driver — with a single, deliberate exception described later in this
subsection. Some directories also carry control files, so a `Dir` may be given
attributes and a
callback object:

```rust
/// A plain directory in the sysfs tree.
pub(crate) struct Dir {
    fields: BranchNodeFields<dyn SysObj, Self>,
    ops: Once<Arc<dyn DirAttrOps>>,
}

/// Callbacks behind the attributes of a [`Dir`], for directories that carry
/// control files (a bus directory, a driver directory).
pub(crate) trait DirAttrOps: Send + Sync + 'static {
    fn show(&self, _name: &str, _w: &mut dyn Write) -> Result<()> { Err(Error::NotFound) }
    fn store(&self, _name: &str, _value: &str) -> Result<()> { Err(Error::NotFound) }
}

impl Dir {
    /// Creates a directory with no attributes, not yet attached anywhere.
    pub(crate) fn new(name: SysStr) -> Arc<Self>;

    /// Creates a directory carrying control files, not yet attached anywhere.
    ///
    /// A directory's attributes are fixed when it is created, so that its
    /// attribute set, like every other in the tree, never changes after it is
    /// built. Only a device's attributes come and go.
    pub(crate) fn with_attrs(
        name: SysStr,
        attrs: &[(&'static str, SysPerms)],
    ) -> Result<Arc<Self>>;

    /// Installs the callbacks behind this directory's control files.
    ///
    /// Separate from `with_attrs` because they usually belong to an object
    /// that needs this directory to exist first. Has no effect after the
    /// first call, and must happen before the directory is attached.
    pub(crate) fn set_ops(&self, ops: Arc<dyn DirAttrOps>);
}
```

`Dir` implements the `systree` traits by hand rather than through the
`inherit_sys_branch_node!` macro, for one reason: the macro would give it a working
`SysBranchNode::remove_child`, and directories of the device model must be edited
only through `SysTreeEdit`.

**One way in for code that is not yet a device.** Some code needs a node where a
class device would go before it is ready to be one. The TSM measurement node is
the case in the tree today: under TDX it publishes
`/sys/devices/virtual/misc/tdx_guest` with nodes of its own. Before this design it
fabricated `/sys/devices`, `virtual` and `misc` itself, which would now collide
with the model's own roots, so the model offers exactly one escape hatch:

```rust
/// Places `node` in the directory `/sys/devices/virtual/<class>`, creating
/// that directory if needed.
///
/// This is for code that puts its own nodes where a class device would go
/// without going through the device model. New code should register a
/// [`ClassDevice`] instead.
pub fn attach_to_virtual_glue_dir(class: &str, node: Arc<dyn SysObj>) -> Result<()>;
```

It is the only public function that puts a node under `/sys/devices` without
registering a device, it goes through the same `GlueDirs` machinery as a class
device would, and it is meant to disappear: converting the TSM node to a `misc`
class device removes the last caller.

**`SymlinkNode`** is a symlink with a literal target, and the two helpers that
create and remove one are where relative targets are computed:

```rust
/// Adds to `dir` a symlink named `name` whose target is the node at
/// `target_path`, expressed relative to `dir` as Linux does.
pub(crate) fn add_link(dir: &dyn SysTreeEdit, name: &str, target_path: &str) -> Result<()> {
    let target = relative_path(&dir.tree_path(), target_path);
    let link = SymlinkNode::new(SysStr::from(name.to_string()), target);
    dir.attach_child(link)
}

/// Removes the symlink named `name` from `dir`, ignoring its absence.
pub(crate) fn remove_link(dir: &dyn SysTreeEdit, name: &str) {
    let _ = dir.detach_child(name);
}
```

Because `add_link` takes a `&dyn SysTreeEdit`, the same call works whether the link
goes into a plain directory (an index) or into a device's own directory (the
`subsystem`, `device` and `driver` links).

**A device is itself a node.** Each of the three device structs implements
`SysObj`, `SysNode` and `SysBranchNode` by delegating to its `DeviceBase`, through
one macro, `impl_device_node!`. The interesting parts are the two attribute
methods, because they are the bridge from a `read()` on a sysfs file to a `show`
callback declared by a bus, a class or a driver. They are shown at the macro body's
own indentation, inside the `impl` the macro writes. `VmWriter` and `VmReader` are
`ostd`'s checked cursors over user memory, which is what a `read()` or `write()`
hands the file system:

```rust
            fn read_attr_at(
                &self,
                name: &str,
                offset: usize,
                writer: &mut VmWriter,
            ) -> aster_systree::Result<usize> {
                if !self.base().is_added() {
                    return Err(aster_systree::Error::IsDead);
                }
                self.base().attrs.show(self, name, offset, writer)
            }

            fn write_attr(&self, name: &str, reader: &mut VmReader) -> aster_systree::Result<usize> {
                if !self.base().is_added() {
                    return Err(aster_systree::Error::IsDead);
                }
                self.base().attrs.store(self, name, reader)
            }
```

The whole read path is therefore: the VFS resolves `/sys/.../vendor` to a sysfs
inode, which holds the `systree` node and the attribute name; sysfs calls
`read_attr_at` on the node, which is the device; the device asks its `AttrTable`
for the erased callback of that name; the callback downcasts the device back to its
concrete type and calls the `show` function the bus declared. A removed device
short-circuits that path with `IsDead`, and its dentry is dropped by the
revalidation hooks.

## Devices

### `DeviceBase`: the spine

Every device, whatever its kind, has the same base. Read the struct below as a
table of contents: four of its field types — `AttrTable`, `GlueDirs`, `DevNum` and
`DevNodeSpec` — get a subsection of their own further down, and the doc comment on
each field says enough to carry you until then.

`SysNodeId` below is `systree`'s per-node identity, unique for the life of the
kernel and never reused, which is what makes a removed-and-re-added name a
different node.

```rust
/// The part of a device that the registration sequence works on.
pub struct DeviceBase {
    id: SysNodeId,
    name: SysStr,
    /// The branch node this device's directory lives in, as `systree` sees
    /// it. Set when the device is attached.
    sys_parent: Once<Weak<dyn SysBranchNode>>,
    /// The same parent, as the registration sequence edits it.
    tree_parent: Once<TreeParent>,
    /// The entries of the device's directory: symlinks, glue directories, and
    /// child devices of any kind.
    children: RwMutex<BTreeMap<SysStr, Arc<dyn SysObj>>>,
    attrs: AttrTable,
    weak_self: Weak<dyn AnyDevice>,
    /// The device this one is reached through, if any.
    parent: Option<Arc<dyn AnyDevice>>,
    devnum: Option<DevNum>,
    state: Mutex<State>,
    /// Glue directories created under this device, one per class of child.
    glue_dirs: GlueDirs,
    /// Which of the symlinks that `add` may create exist, so that `teardown`
    /// removes only links this device made.
    links: Mutex<Links>,
    /// Devices whose parent is this one.
    child_devices: Mutex<Vec<Weak<dyn AnyDevice>>>,
    /// The `/dev` node created for this device, kept so that the uevent
    /// variables agree with it and so that it can be deleted.
    devnode: Mutex<Option<DevNodeSpec>>,
}
```

Three fields deserve comment.

`sys_parent` and `tree_parent` are the same parent seen twice: `systree` needs a
`Weak<dyn SysBranchNode>` to compute paths, and the registration sequence needs the
crate-private editing view, which may be a plain directory or another device:

```rust
enum TreeParent {
    Dir(Weak<Dir>),
    Device(Weak<dyn AnyDevice>),
}

impl TreeParent {
    /// Runs `f` with the crate-private editing view of the parent, if the
    /// parent is still alive.
    fn with_edit<R>(&self, f: impl FnOnce(&dyn SysTreeEdit) -> R) -> Option<R> {
        match self {
            TreeParent::Dir(dir) => dir.upgrade().map(|dir| f(dir.as_ref())),
            TreeParent::Device(dev) => dev.upgrade().map(|dev| f(dev.base())),
        }
    }
}
```

`links` records which of the four optional symlinks this device actually created.
Teardown consults it instead of deleting by name, because a name in an index
directory may belong to a *different* device: if two class devices with the same
name are registered under different parents, the second fails at the index link,
and its teardown must not delete the first one's entry.

`state` is the life cycle, and it is what makes registration one-shot:

```rust
enum State {
    /// Built, not yet in the tree.
    Initialized,
    /// Registration in progress; the directory exists.
    Adding,
    /// Registered.
    Added,
    /// Removed; the object may still be referenced but is dead.
    Removed,
}
```

`is_added` treats `Adding` as added, which is deliberate: a driver's `probe` runs
inside the parent's `add`, and it must be able to register children of a device
that is still mid-registration.

```rust
    /// Returns whether the device is currently registered.
    pub fn is_added(&self) -> bool {
        // A device counts as added as soon as its directory exists, so that a
        // driver's `probe`, which runs inside `add`, can register children.
        matches!(*self.state.lock(), State::Adding | State::Added)
    }
```

### `AnyDevice`: the erased view

```rust
/// The erased view of any device.
///
/// This is what the registration sequence, the sysfs tree, and parents see.
/// Drivers and classes never need it: they receive the concrete
/// [`BusDevice`] or [`ClassDevice`].
///
/// The trait is sealed twice over, through [`Container`](crate::Container)
/// and through the crate-private `DeviceInternals`: only the three device
/// structs of this crate implement it.
pub trait AnyDevice: crate::Container + DeviceInternals {
    /// Returns the shared base.
    fn base(&self) -> &DeviceBase;

    /// Returns the subsystem that owns the device.
    fn subsystem(&self) -> Subsystem;

    /// Returns the name of the bound driver, for bus devices.
    fn driver_name(&self) -> Option<String>;

    /// Returns a strong reference to this device as a trait object.
    fn to_arc(&self) -> Arc<dyn AnyDevice> {
        self.base()
            .weak_self
            .upgrade()
            .expect("a device is only reachable through an `Arc`")
    }

    /// Returns whether this is a class device.
    fn is_class_device(&self) -> bool {
        self.subsystem().kind() == SubsystemKind::Class
    }
}
```

What the registration sequence needs beyond that is plumbing, not a service to
callers, so it lives on a second trait that never leaves the crate — which is also
the second half of the seal:

```rust
mod internals {
    /// What the registration sequence asks a device for.
    ///
    /// These are plumbing, not a service to callers, so the trait is private
    /// to the crate; it is also what seals [`super::AnyDevice`].
    pub trait DeviceInternals {
        fn type_name(&self) -> Option<&'static str>;
        fn attr_groups(&self) -> Vec<TyErasedAttr>;
        fn subsystem_uevent(&self, vars: &mut UeventVars);
        fn devnode_override(&self) -> Option<DevNode>;
        fn wants_device_link(&self) -> bool { true }
    }
}
```

`TyErasedAttr` in that listing is an attribute whose callbacks have been made to take
`&dyn AnyDevice`; *Attributes* below is where it is built and stored.

Because `DeviceInternals` is `pub` inside a private module and `Container` is
sealed, no code outside the crate can implement `AnyDevice`. That is what licenses
the downcasts described later: the set of implementations is closed and known.

### The three device structs

```rust
/// A device enumerated on bus `B`.
///
/// Dereferences to the bus-specific payload `B::Device`.
pub struct BusDevice<B: Bus> {
    base: DeviceBase,
    bus: Arc<BusHandle<B>>,
    payload: B::Device,
    declared: DeclaredParts<Self>,
    driver: RwMutex<Option<Arc<DriverHandle<B>>>>,
    /// Serializes binding and unbinding, as Linux's `dev->mutex` does.
    bind_lock: Mutex<()>,
    weak: Weak<Self>,
}

/// A device in class `C`: the interface user space sees.
pub struct ClassDevice<C: Class> {
    base: DeviceBase,
    class: Arc<ClassHandle<C>>,
    payload: C::Device,
    declared: DeclaredParts<Self>,
    weak: Weak<Self>,
}

/// A device with neither bus nor class, such as a host bridge or a firmware
/// root.
///
/// It has no attributes beyond `uevent`, is listed in no index, and sends no
/// uevents; it exists to be the parent of other devices.
pub struct BareDevice {
    base: DeviceBase,
}
```

Each holds its subsystem handle strongly, which is why a bus outlives its devices,
and each dereferences to its payload:

```rust
impl<B: Bus> Deref for BusDevice<B> {
    type Target = B::Device;

    fn deref(&self) -> &B::Device {
        &self.payload
    }
}
```

`DeclaredParts<D>` is the pair of per-type things a bus or class device carries, kept
in one place so that the two structs do not duplicate the logic:

```rust
/// The per-type parts a bus or class device carries: its device type and its
/// own attributes.
pub(crate) struct DeclaredParts<D: ?Sized + 'static> {
    dev_type: Option<&'static DeviceType<D>>,
    own_attrs: &'static [Attr<D>],
}

impl<D: AnyDevice> DeclaredParts<D> {
    /// Erases the subsystem's, the type's, and the device's own attributes,
    /// in that order.
    fn attr_groups(&self, subsystem_attrs: &[Attr<D>]) -> Vec<TyErasedAttr> {
        let mut attrs = TyErasedAttr::from_typed_slice(subsystem_attrs);
        if let Some(t) = self.dev_type {
            attrs.extend(TyErasedAttr::from_typed_slice(t.attrs));
        }
        attrs.extend(TyErasedAttr::from_typed_slice(self.own_attrs));
        attrs
    }
    // type_name, type_uevent, type_devnode, has_device_link
}
```

**One caveat about `Deref`.** A `BusDevice<B>` has the `systree` methods `name()`,
`path()` and `parent()`, and it dereferences to a payload that may have methods of
the same names. Which one `dev.name()` resolves to depends on which traits are in
scope at the call site. Driver code should write `dev.base().name()` for the tree
name and `dev.payload()` for the payload; the ambiguity is a nuisance, not a
soundness problem, and it is the price of the `Deref` convenience that makes
`dev.vendor` work in an attribute callback.

### The builder

All bus and class devices are built by one generic builder. Its three parameters
are always determined by the device type, so the crate exports an alias for each:

```rust
/// Builds a bus or class device.
///
/// Obtained from `BusDevice::builder` or `ClassDevice::builder`. The built
/// device is not in the tree until `add` is called.
///
/// `H` is the subsystem handle, `P` the payload, and `D` the device type the
/// attributes apply to.
pub struct DeviceBuilder<H, P, D: ?Sized + 'static> {
    handle: H,
    payload: P,
    name: SysStr,
    parent: Option<Arc<dyn AnyDevice>>,
    devnum: Option<DevNum>,
    dev_type: Option<&'static DeviceType<D>>,
    attrs: &'static [Attr<D>],
}

pub type BusDeviceBuilder<B> = DeviceBuilder<Arc<BusHandle<B>>, <B as Bus>::Device, BusDevice<B>>;
pub type ClassDeviceBuilder<C> = DeviceBuilder<Arc<ClassHandle<C>>, <C as Class>::Device, ClassDevice<C>>;

impl<H, P, D: ?Sized + 'static> DeviceBuilder<H, P, D> {
    pub fn parent(mut self, parent: Arc<dyn AnyDevice>) -> Self { /* ... */ }
    pub fn devnum(mut self, devnum: DevNum) -> Self { /* ... */ }
    pub fn dev_type(mut self, dev_type: &'static DeviceType<D>) -> Self { /* ... */ }
    pub fn attrs(mut self, attrs: &'static [Attr<D>]) -> Self { /* ... */ }
}
```

`build` is written once per device kind, because it is the one place that knows how
to fill the struct. It uses `Arc::new_cyclic` so that the core can hold a `Weak`
to the finished device, which is what `to_arc` upgrades:

```rust
impl<B: Bus> BusDeviceBuilder<B> {
    /// Builds the device. It is not registered until `add` is called.
    pub fn build(self) -> Arc<BusDevice<B>> {
        let declared = self.declared_parts();
        Arc::new_cyclic(|weak: &Weak<BusDevice<B>>| {
            let weak_self: Weak<dyn AnyDevice> = weak.clone();
            BusDevice {
                base: DeviceBase::new(self.name, self.parent, self.devnum, weak_self),
                bus: self.handle,
                payload: self.payload,
                declared,
                driver: RwMutex::new(None),
                bind_lock: Mutex::new(()),
                weak: weak.clone(),
            }
        })
    }
}
```

A bare device needs no payload and no handle, so it has two named constructors
rather than a builder, which keeps an unlabelled `None` out of call sites:

```rust
impl BareDevice {
    /// Creates a bare device at the top of `/sys/devices`.
    pub fn new_root(name: impl Into<SysStr>) -> Arc<Self>;

    /// Creates a bare device under `parent`.
    pub fn with_parent(name: impl Into<SysStr>, parent: Arc<dyn AnyDevice>) -> Arc<Self>;
}
```

### Device types

```rust
/// A finer kind within one bus or class, such as `disk` versus `partition`
/// in the `block` class.
///
/// A type contributes attributes, uevent variables, and a `devnode` policy to
/// every device that carries it, and is reported to user space as `DEVTYPE`.
pub struct DeviceType<D: ?Sized + 'static> {
    /// The name reported as `DEVTYPE`.
    pub name: &'static str,
    /// Attributes every device of this type gets.
    pub attrs: &'static [Attr<D>],
    /// Adds type-specific uevent variables.
    pub uevent: Option<fn(&D, &mut UeventVars)>,
    /// Overrides the `/dev` node name or mode.
    pub devnode: Option<fn(&D) -> Option<DevNode>>,
    /// Whether a class device of this type gets the `device` symlink to its
    /// parent. Linux omits the link for one type only: block partitions.
    pub has_device_link: bool,
}

impl<D: ?Sized + 'static> DeviceType<D> {
    /// Creates a type with a name and nothing else.
    pub const fn named(name: &'static str) -> Self;
}
```

A device type is a `static` a builder points at, so it costs one pointer per device
and its callbacks are ordinary function pointers. `has_device_link` exists for one
Linux behavior: block partitions are class devices with a class-device parent, and
they are the only kind that does *not* get a `device` symlink
(`device_is_not_partition` in `device_add_class_symlinks`).

## Subsystems: buses and classes

### The `Bus` trait

```rust
/// A kind of bus.
pub trait Bus: Sized + Send + Sync + 'static {
    /// The bus name: the directory under `/sys/bus`.
    const NAME: &'static str;

    /// What every device on this bus carries: its address, identifiers,
    /// and resources.
    type Device: Send + Sync + 'static;

    /// What a driver declares to say which devices it accepts.
    type MatchData: Send + Sync + 'static;

    /// Decides whether a driver's match data accepts a device.
    fn matches(&self, dev: &Self::Device, data: &Self::MatchData) -> bool;

    /// Attributes every device on this bus gets.
    fn dev_attrs(&self) -> &'static [Attr<BusDevice<Self>>] { &[] }

    /// Adds bus-specific uevent variables, typically `MODALIAS`.
    fn uevent(&self, _dev: &BusDevice<Self>, _vars: &mut UeventVars) {}
}
```

`MatchData` is what makes matching data-driven without being stringly typed: a PCI
bus would make it a table of vendor and device ids, virtio a device type and vendor
pair, and `matches` a pure function of the two. Nothing in the model interprets it.

Registering a bus creates its directory, its two control files, the `devices`
index and the `drivers` directory that will hold one subdirectory per registered
driver, and hands back a handle:

```rust
/// Registers a bus, creating `/sys/bus/<name>/{devices,drivers}`.
pub fn register_bus<B: Bus>(bus: B) -> Result<Arc<BusHandle<B>>> {
    let dir = Dir::with_attrs(
        SysStr::from(B::NAME),
        &[
            ("drivers_autoprobe", SysPerms::DEFAULT_RW_ATTR_PERMS),
            ("drivers_probe", SysPerms::OWNER_W),
        ],
    )?;
    let devices_dir = Dir::new(SysStr::from("devices"));
    let drivers_dir = Dir::new(SysStr::from("drivers"));
    dir.attach_child(devices_dir.clone())?;
    dir.attach_child(drivers_dir.clone())?;

    let handle = Arc::new_cyclic(|weak| BusHandle {
        bus,
        dir: dir.clone(),
        devices_dir,
        drivers_dir,
        devices: Mutex::new(Vec::new()),
        drivers: Mutex::new(Vec::new()),
        autoprobe: AtomicBool::new(true),
        weak: weak.clone(),
    });
    dir.set_ops(Arc::new(BusDirOps { bus: handle.weak.clone() }));
    crate::registry().bus_root().attach_child(dir)?;
    crate::registry().keep_subsystem(handle.clone());
    Ok(handle)
}
```

`BusHandle<B>` is the bus's runtime state: the bus value itself, its directories,
its device and driver lists, and the autoprobe switch. It is what a driver
registers with and what a device is built against, so a `BusDevice<B>` cannot exist
without a registered bus `B`.

### The `Class` trait

```rust
/// A kind of class.
pub trait Class: Sized + Send + Sync + 'static {
    /// The class name: the directory under `/sys/class`.
    const NAME: &'static str;

    /// What every device in this class carries.
    type Device: Send + Sync + 'static;

    /// Overrides the `/dev` node name or mode of a device.
    fn devnode(&self, _dev: &ClassDevice<Self>) -> Option<DevNode> { None }

    /// Attributes every device in this class gets.
    fn dev_attrs(&self) -> &'static [Attr<ClassDevice<Self>>] { &[] }

    /// Adds class-specific uevent variables.
    fn uevent(&self, _dev: &ClassDevice<Self>, _vars: &mut UeventVars) {}

    /// Whether a device of this class placed under a class device still gets
    /// its own glue directory. Linux does this for classes that define a
    /// sysfs namespace type, such as `net`, whose glue directory is what
    /// scopes the names to a namespace.
    const KEEPS_GLUE_DIR: bool = false;
}
```

`register_class` mirrors `register_bus`:

```rust
/// Registers a class and creates its directory under `/sys/class`.
pub fn register_class<C: Class>(class: C) -> Result<Arc<ClassHandle<C>>>;
```

`ClassHandle<C>` is the class's runtime state, the counterpart of `BusHandle<B>`:
the class value itself, its directory under `/sys/class`, its member list, its
registered interfaces, and the membership lock that serializes the two. A
`ClassDevice<C>` is built against it, so a class device cannot exist without a
registered class.

A class has no matching rule and no drivers; it has naming policy and membership.
Membership is observable, and that deserves a paragraph, because Asterinas is
already paying for not having it.

Some code has to act on *every* member of a class without being the code that
creates them. Today there is nowhere for such a consumer to look, so each subsystem
keeps a device list of its own — `aster_block::DEVICE_REGISTRY`,
`aster_console::console_device_table`, `EVDEV_DEVICES` — and every producer must
remember to call every consumer by hand. A class interface inverts that: the class
is the one place that knows its members, and a consumer subscribes to it.

The strongest evidence that this is needed is that one subsystem has already built
it. `aster-input` has `InputHandlerClass`, which is a class interface under another
name, replayed in both directions: `register_handler_class` connects a new handler
to every device that already exists, and `register_device` connects every existing
handler to a new device. That is how evdev gets an `/dev/input/eventN` for every
input device without virtio-input calling it. The mechanism is right; it is just
private to one subsystem, and rebuilt from scratch by anyone who needs it again.

Two consumers that need it and do not have it:

- **A layer built on top of a class.** When a disk joins the `block` class,
  something must read its partition table and register a device per partition.
  That code belongs to the partition layer, not to virtio-blk or NVMe, and it has
  to run for disks from every driver. Today partition scanning is a one-shot sweep
  rather than a subscription, which is why a disk that appears later is missed.
- **A chooser.** The console picks among the members of the `tty` class by walking
  `all_devices()`, and would rather be told as they appear.

Linux does the same thing with `class_interface_register`, which the SCSI core uses
this way through `scsi_register_interface`.

```rust
/// Callbacks run for every device that joins or leaves a class.
pub trait ClassInterface<C: Class>: Send + Sync + 'static {
    /// A device has joined the class.
    fn add_dev(&self, dev: &Arc<ClassDevice<C>>);

    /// A device is leaving the class.
    fn remove_dev(&self, _dev: &Arc<ClassDevice<C>>) {}
}
```

`register_interface` replays `add_dev` for members that already exist, so a late
subscriber sees the full set (R13). That replay is what makes initialization order
free: the partition layer sees the same disks whether it starts before or after the
disk drivers, and it sees each disk exactly once, because the replay runs under the
same lock that admits new members.

```rust
    /// Registers an interface and replays `add_dev` for the current members.
    ///
    /// `add_dev` runs under the class's membership lock and must not add or
    /// remove devices of this class.
    pub fn register_interface(&self, interface: Arc<dyn ClassInterface<C>>) {
        let _guard = self.membership.lock();
        self.interfaces.lock().push(interface.clone());
        for dev in self.devices() {
            interface.add_dev(&dev);
        }
    }
```

`unregister_interface` is the mirror image: it drops the interface and then calls
`remove_dev` for every current member, so a subscriber that goes away sees each
device leave exactly once, just as it saw each one arrive.

### The one erased subsystem trait

The registration sequence works on `dyn AnyDevice` and must ask that device's
subsystem for its directories and tell it when the device arrives or leaves. Both
handles therefore implement one small object-safe trait, which is crate-private and
reachable only through the `Subsystem` value a device reports:

```rust
/// What the registration sequence needs from a bus or class handle.
pub(crate) trait SubsystemOps: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    /// The directory the `subsystem` link points at: `/sys/bus/<b>` for a bus,
    /// `/sys/class/<c>` for a class.
    fn dir(&self) -> Arc<Dir>;

    /// The directory the index entry goes in: `/sys/bus/<b>/devices` for a bus,
    /// and for a class `/sys/class/<c>` — the same directory as `dir`.
    fn index_dir(&self) -> Arc<Dir>;
    fn keeps_glue_dir(&self) -> bool;

    /// Records a device that has just been registered and acts on it: a bus
    /// probes for a driver, a class notifies its interfaces.
    fn on_added(&self, dev: &Arc<dyn AnyDevice>);

    /// Forgets a device that is being removed: a bus unbinds it, a class
    /// notifies its interfaces.
    fn on_removed(&self, dev: &Arc<dyn AnyDevice>);
}

/// The subsystem that owns a device. A device has exactly one.
#[derive(Clone)]
pub struct Subsystem {
    kind: SubsystemKind,
    /// The bus or class handle; `None` for a bare device.
    ops: Option<Arc<dyn SubsystemOps>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubsystemKind { Bus, Class, Bare }
```

`Subsystem` is a small value type, not a trait object. Its `kind` says which of the
three cases this is; `name` returns the bus or class name, or `None` for a bare
device; `dir` and `index_dir` forward to the handle and return `None` for a bare
device, which is why `add_steps` writes `if let Some(dir) = subsystem.dir()` where
`SubsystemOps` returns a bare `Arc<Dir>`; `ops` hands back the handle itself; and
`keeps_glue_dir` forwards the class constant that placement consults.

`on_added` and its mirror `on_removed` are one of the design's two downcast sites;
the other is the attribute table, below. Both are where the closed set of device
types pays off. `as_any()` comes from `systree`'s `SysObj`, which every device
implements through `impl_device_node!`; it is the one hatch that turns a
`&dyn AnyDevice` back into a concrete device struct. `dev.this()` upgrades a
device's weak self-reference into an `Arc`, which registration needs because a
device is only ever reachable through one:

```rust
    fn on_added(&self, dev: &Arc<dyn AnyDevice>) {
        let Some(dev) = dev.as_any().downcast_ref::<BusDevice<B>>() else {
            // Only a `BusDevice<B>` reports this bus as its subsystem.
            ostd::error!("a device of another type was added to bus {}", B::NAME);
            return;
        };
        let dev = dev.this();
        self.devices.lock().push(dev.clone());
        if self.autoprobe.load(Ordering::Relaxed) {
            // No driver is not an error at registration time.
            let _ = self.probe(&dev);
        }
    }
```

The downcast cannot fail: a `BusDevice<B>` is the only device that reports
`Subsystem::bus(self)`, and the trait is sealed, so no fourth device type can be
introduced to violate that. The `else` arm logs rather than panicking because the
invariant is one the crate maintains, not one a caller can break.

## Attributes

An attribute is one file in a device directory. The problem the model has to solve
is that five different layers contribute attributes to the same directory, each
knowing a different concrete type, while the directory itself knows none of them.

### Declaration is typed

```rust
/// Produces the text of an attribute.
pub type ShowFn<D> = fn(&D, &mut dyn Write) -> Result<()>;

/// Consumes the text written to an attribute.
pub type StoreFn<D> = fn(&D, &str) -> Result<()>;

/// A statically declared attribute of devices of type `D`.
pub struct Attr<D: ?Sized> {
    name: &'static str,
    perms: SysPerms,
    show: Option<ShowFn<D>>,
    store: Option<StoreFn<D>>,
}

impl<D: ?Sized> Attr<D> {
    pub const fn ro(name: &'static str, show: ShowFn<D>) -> Self;
    pub const fn rw(name: &'static str, show: ShowFn<D>, store: StoreFn<D>) -> Self;
    pub const fn wo(name: &'static str, store: StoreFn<D>) -> Self;
}
```

The callbacks are plain function pointers, not closures, so tables of attributes
are `const` and live in static memory, exactly as Linux's `attribute_group` tables
do. The three constructors are what keep permissions and callbacks consistent: a
read-only attribute has no `store`, a write-only one has no `show`, and there is no
way to build an inconsistent pair.

Every layer declares its attributes against the device type it applies to:
`Bus::dev_attrs` returns `&[Attr<BusDevice<Self>>]`, `Class::dev_attrs` returns
`&[Attr<ClassDevice<Self>>]`, `DeviceType<D>` carries `&[Attr<D>]`, and the builder
takes `&'static [Attr<D>]`. Handing a class attribute to a bus device is a type
error.

### Storage is erased

```rust
type TyErasedShowFn = Arc<dyn Fn(&dyn AnyDevice, &mut dyn Write) -> Result<()> + Send + Sync>;
type TyErasedStoreFn = Arc<dyn Fn(&dyn AnyDevice, &str) -> Result<()> + Send + Sync>;

/// An attribute whose callbacks take `&dyn AnyDevice`.
#[derive(Clone)]
pub struct TyErasedAttr {
    name: &'static str,
    perms: SysPerms,
    show: Option<TyErasedShowFn>,
    store: Option<TyErasedStoreFn>,
}

impl TyErasedAttr {
    /// Erases an attribute declared for the concrete device type `D`.
    pub(crate) fn from_typed<D: AnyDevice>(attr: &Attr<D>) -> Self {
        let show = attr.show.map(|show| -> TyErasedShowFn {
            Arc::new(move |dev: &dyn AnyDevice, w: &mut dyn Write| {
                let dev = dev.as_any().downcast_ref::<D>().ok_or(Error::Attribute)?;
                show(dev, w)
            })
        });
        // ... the same for `store`
    }
}
```

The downcast inside is safe for the same reason as the subsystem callbacks: only
`BusDevice<B>::attr_groups` produces `Attr<BusDevice<B>>` entries, and it produces
them for itself.

The per-device table pairs the erased callbacks with the `SysAttrSet` that sysfs
reads:

```rust
/// The attributes of one device: the `SysTree` attribute set that sysfs
/// lists, plus the callbacks behind each entry.
pub(crate) struct AttrTable {
    inner: RwMutex<TableInner>,
}

struct TableInner {
    /// What sysfs lists. Replaced wholesale, never edited.
    set: Arc<SysAttrSet>,
    /// What serves a read or a write of each listed attribute.
    ops: BTreeMap<SysStr, TyErasedAttr>,
}

impl AttrTable {
    pub(crate) fn new() -> Self {
        Self {
            inner: RwMutex::new(TableInner {
                set: SysAttrSet::empty().clone(),
                ops: BTreeMap::new(),
            }),
        }
    }

    /// The attribute set, in the form sysfs consumes.
    pub(crate) fn set(&self) -> Arc<SysAttrSet> {
        self.inner.read().set.clone()
    }

    /// Adds attributes. Fails if any name is already present or is repeated
    /// within `attrs`, or if the ID space is exhausted; in every case none of
    /// the attributes is added.
    ///
    /// Repetition has to be caught here rather than left to the set builder,
    /// which treats a repeated name as a no-op: registration hands the layers
    /// of one device over as a single batch, so without this check a bus that
    /// declared an attribute named `uevent` would quietly take over the core's
    /// callback while sysfs went on showing the core's entry.
    pub(crate) fn add(&self, attrs: Vec<TyErasedAttr>) -> Result<()> {
        let mut inner = self.inner.write();
        let mut seen = BTreeSet::new();
        if attrs
            .iter()
            .any(|attr| inner.ops.contains_key(attr.name) || !seen.insert(attr.name))
        {
            return Err(Error::NameConflict);
        }

        let mut builder = SysAttrSetBuilder::from_set(&inner.set);
        for attr in &attrs {
            builder.add(SysStr::from(attr.name), attr.perms);
        }
        // The new set is built to one side, so an exhausted ID space is
        // reported here with the table still exactly as it was.
        inner.set = Arc::new(builder.build()?);
        for attr in attrs {
            inner.ops.insert(SysStr::from(attr.name), attr);
        }
        Ok(())
    }

    /// Removes attributes by name. Names that are absent are ignored.
    pub(crate) fn remove(&self, names: &[&'static str]) {
        let mut inner = self.inner.write();
        let mut builder = SysAttrSetBuilder::from_set(&inner.set);
        for name in names {
            inner.ops.remove(*name);
            builder.remove(name);
        }
        let set = builder
            .build()
            .expect("a builder that only removes attributes cannot fail");
        inner.set = Arc::new(set);
    }
}
```

Adding is all-or-nothing, which matters because registration adds the layers of one
device as a single group: a name collision must not leave half a group behind. A
name repeated *within* the group is a collision too, and has to be caught here
rather than left to the set builder, which treats a repeated name as a no-op. Were
it not caught, a bus that declared an attribute named `uevent` would take over the
core's callback while sysfs went on listing the core's entry, with no error.
The published set is built to one side and only then swapped in, so a failure part
way through leaves the table exactly as it was, and a concurrent reader never sees
a set with half a group in it.

Serving a read clones the callback out of the table and releases the lock before
calling it:

```rust
    /// Runs the `show` callback of `name`.
    ///
    /// The callback runs after the lock is released, so that no attribute
    /// callback holds a lock of the device model. The cost is that a call
    /// already in flight can finish after its attribute has been removed; it
    /// stays safe because the caller holds the device alive. Linux instead
    /// drains active references in `kernfs_drain`.
    pub(crate) fn show(
        &self,
        dev: &dyn AnyDevice,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
    ) -> aster_systree::Result<usize> {
        let show = {
            let ops = self.ops.read();
            let attr = ops.get(name).ok_or(aster_systree::Error::NotFound)?;
            attr.show.clone().ok_or(aster_systree::Error::PermissionDenied)?
        };
        let mut printer = VmPrinter::new_skip(writer, offset);
        show(dev, &mut printer).map_err(aster_systree::Error::from)?;
        Ok(printer.bytes_written())
    }
```

`VmPrinter` is a `core::fmt::Write` adapter over a `VmWriter`, and
`VmPrinter::new_skip` is what makes `offset` work: the callback always renders the
whole value, and the printer throws away the first `offset` bytes. A read of a
large attribute in several chunks therefore re-renders it once per chunk, which is
what sysfs does too.

Releasing the lock before the callback is deliberate; the discussion at the end of
this document explains why holding it across the callback would be worse.

### The two core attributes

Every device gets `uevent`, and every device with a number also gets `dev`. They
are declared like any other attribute, except that they are typed against the
erased view, because they work for all three device structs:

```rust
const CORE_ATTRS: &[Attr<dyn AnyDevice>] = &[Attr::rw("uevent", show_uevent, store_uevent)];
const DEV_ATTR: Attr<dyn AnyDevice> = Attr::ro("dev", show_dev);

fn show_uevent(dev: &dyn AnyDevice, w: &mut dyn Write) -> Result<()> {
    dev_uevent_vars(dev).write_lines(w)?;
    Ok(())
}

fn store_uevent(dev: &dyn AnyDevice, value: &str) -> Result<()> {
    let action: UeventAction = value.parse()?;
    emit_uevent(dev, action);
    Ok(())
}

fn show_dev(dev: &dyn AnyDevice, w: &mut dyn Write) -> Result<()> {
    let devnum = dev.base().devnum().ok_or(Error::NoDevNum)?;
    writeln!(w, "{}", devnum)?;
    Ok(())
}
```

`TyErasedAttr::from_dyn` wraps these without a downcast, since their callbacks
already take `&dyn AnyDevice`. It is `store_uevent`, above, that makes writing an
action to a `uevent` file re-emit an event, which is what `udevadm trigger` relies
on.

### The five layers

In order of addition: the core (`uevent` always, `dev` when the device has a
number), the subsystem, the device type, the device itself, and, at bind time, the
driver. The first four are added during registration, in the order above; the fifth
is added after `probe` succeeds and removed before the driver's `remove` runs.

## Placement: deriving the view from the topology

`place` is the only function that decides where a device's directory goes. It
implements Linux's `get_device_parent` and records, in `tree_parent`, how to find
that directory again at removal:

```rust
/// Decides which directory a device's directory goes into, and attaches it
/// there.
///
/// - A class device under a class device sits directly inside it.
/// - A class device under any other device sits in a glue directory named
///   after its class, created inside the parent.
/// - A class device with no parent sits in a glue directory under
///   `/sys/devices/virtual`.
/// - A bus device or a bare device sits directly under its parent, or at the
///   top of `/sys/devices` if it has none.
fn place(dev: &Arc<dyn AnyDevice>) -> Result<()> {
    let parent = dev.base().parent();
    if let Some(parent) = parent
        && !parent.base().is_added()
    {
        return Err(Error::ParentNotAdded);
    }

    let subsystem = dev.subsystem();
    let class = subsystem.name().unwrap_or_default();
    let child: Arc<dyn SysObj> = dev.clone();
    let tree_parent = match (subsystem.kind(), parent) {
        (SubsystemKind::Class, Some(parent)) => {
            if uses_glue_dir(&subsystem, parent) {
                let dir = parent
                    .base()
                    .glue_dirs
                    .attach_into(class, parent.base(), child)?;
                TreeParent::Dir(Arc::downgrade(&dir))
            } else {
                parent.base().attach_child(child)?;
                TreeParent::Device(Arc::downgrade(parent))
            }
        }
        (SubsystemKind::Class, None) => {
            let dir = registry().attach_into_virtual_glue_dir(class, child)?;
            TreeParent::Dir(Arc::downgrade(&dir))
        }
        (_, Some(parent)) => {
            parent.base().attach_child(child)?;
            TreeParent::Device(Arc::downgrade(parent))
        }
        (_, None) => {
            let root = registry().devices_root();
            root.attach_child(child)?;
            TreeParent::Dir(Arc::downgrade(root))
        }
    };
    dev.base().tree_parent.call_once(|| tree_parent);
    Ok(())
}

/// Returns whether a class device under `parent` is placed in a glue
/// directory, as opposed to directly inside a class-device parent.
fn uses_glue_dir(subsystem: &Subsystem, parent: &Arc<dyn AnyDevice>) -> bool {
    let sits_inside_parent = parent.is_class_device();
    !sits_inside_parent || subsystem.keeps_glue_dir()
}
```

### Glue directories

A glue directory is created by the first device of a class placed under a given
parent and dropped when the last one leaves. Both transitions have to be atomic
with respect to each other, or a sibling can be attached into a directory that is
being removed. Linux serializes them with a global `gdp_mutex`; the model gives
each container its own:

```rust
/// Glue directories owned by one container, one per class of child.
///
/// A glue directory is created when the first device of a class is placed
/// under the container and dropped when the last one leaves. Both
/// transitions happen under one lock, so a device cannot be attached into a
/// glue directory that is being dropped (Linux's `gdp_mutex` serves the same
/// purpose).
pub(crate) struct GlueDirs {
    dirs: Mutex<BTreeMap<SysStr, Arc<Dir>>>,
}

impl GlueDirs {
    /// Attaches `child` into the glue directory `name` under `owner`,
    /// creating the directory if needed, and returns that directory.
    pub(crate) fn attach_into(
        &self,
        name: &str,
        owner: &dyn SysTreeEdit,
        child: Arc<dyn SysObj>,
    ) -> Result<Arc<Dir>> {
        let mut dirs = self.dirs.lock();
        if let Some(dir) = dirs.get(name) {
            dir.attach_child(child)?;
            return Ok(dir.clone());
        }
        let dir = Dir::new(SysStr::from(name.to_string()));
        owner.attach_child(dir.clone())?;
        if let Err(e) = dir.attach_child(child) {
            let _ = owner.detach_child(name);
            return Err(e);
        }
        dirs.insert(SysStr::from(name.to_string()), dir.clone());
        Ok(dir)
    }

    /// Drops the glue directory `name` under `owner` if it is now empty.
    pub(crate) fn drop_if_empty(&self, name: &str, owner: &dyn SysTreeEdit) {
        let mut dirs = self.dirs.lock();
        if let Some(dir) = dirs.get(name)
            && !dir.has_children()
        {
            let _ = owner.detach_child(name);
            dirs.remove(name);
        }
    }
}
```

Note that `attach_into` does the attach itself rather than returning the directory
for the caller to fill: that is what keeps the lock held across "find or create"
and "attach", which is the whole point.

## Registration and removal

This is the center of the model: one function that produces every artifact, and one
that removes them in reverse. Three names in the code below belong to later
subsections: `devnode_spec` computes the `/dev` node a device asks for,
`hooks::create_devnode` is the call into devtmpfs, and `emit_uevent` builds and
hands off an event. All three are covered under *Device numbers, `/dev` nodes and
uevents*.

### `add`

```rust
/// Registers a device: the Asterinas counterpart of Linux's `device_add`.
///
/// The steps, in order: place the directory, add the core attributes, add the
/// attributes of the subsystem, the type, and the device, create the
/// `subsystem`, `device`, and index symlinks, publish the device number and
/// the `/dev` node, announce the device, let the subsystem act (a bus probes
/// for a driver; a class notifies its interfaces), and link the device into
/// its parent. On failure everything done so far is undone.
///
/// A failed registration is final: the device ends up removed and cannot be
/// added again.
pub fn add<D: AnyDevice + ?Sized>(dev: &Arc<D>) -> Result<()> {
    add_erased(dev.to_arc())
}

fn add_erased(this: Arc<dyn AnyDevice>) -> Result<()> {
    let base = this.base();
    if base.name().is_empty() || base.name().contains('/') || base.name().contains('\0') {
        return Err(Error::InvalidName);
    }
    {
        let mut state = base.state.lock();
        if *state != State::Initialized {
            return Err(Error::AlreadyAdded);
        }
        *state = State::Adding;
    }

    match add_steps(&this) {
        Ok(()) => {
            *base.state.lock() = State::Added;
            Ok(())
        }
        Err(e) => {
            teardown(&this, Teardown::Silent);
            *base.state.lock() = State::Removed;
            Err(e)
        }
    }
}
```

`add` takes `&Arc<D>` rather than `Arc<D>` because the erased view it needs comes
from the device's own weak self-pointer; the caller keeps its handle. The generic
parameter is `?Sized` so that `add(&child)` works when `child` is already an
`Arc<dyn AnyDevice>`, which is what a driver's `remove` has.

The eight steps:

```rust
fn add_steps(this: &Arc<dyn AnyDevice>) -> Result<()> {
    let base = this.base();
    let subsystem = this.subsystem();

    // 1. Place the directory.
    place(this)?;

    // 2. Core attributes.
    let mut attrs: Vec<TyErasedAttr> = CORE_ATTRS.iter().map(TyErasedAttr::from_dyn).collect();
    if base.devnum().is_some() {
        attrs.push(TyErasedAttr::from_dyn(&DEV_ATTR));
    }
    // 3. Subsystem, type, and device attributes.
    attrs.extend(this.attr_groups());
    base.attrs.add(attrs)?;

    // 4. Links: `subsystem`, `device`, and the index entry.
    let path = base.tree_path();
    if let Some(dir) = subsystem.dir() {
        add_link(base, "subsystem", &SysObj::path(dir.as_ref()))?;
        base.links.lock().subsystem = true;
    }
    if let (SubsystemKind::Class, Some(parent)) = (subsystem.kind(), base.parent())
        && this.wants_device_link()
    {
        add_link(base, "device", &parent.base().tree_path())?;
        base.links.lock().device = true;
    }
    if let Some(index) = subsystem.index_dir() {
        add_link(index.as_ref(), base.name(), &path)?;
        base.links.lock().index = true;
    }

    // 5. Device number: `/sys/dev` entry and `/dev` node.
    if let Some(devnum) = base.devnum() {
        let index = registry().dev_index(devnum.kind());
        add_link(index.as_ref(), &devnum.to_string(), &path)?;
        base.links.lock().dev_index = true;
        let spec = devnode_spec(this.as_ref(), devnum);
        hooks::create_devnode(spec.request.clone())?;
        *base.devnode.lock() = Some(spec);
    }

    // 6. Announce.
    emit_uevent(this.as_ref(), UeventAction::Add);

    // 7. Let the subsystem act.
    if let Some(ops) = subsystem.ops() {
        ops.on_added(this);
    }

    // 8. Link into the parent.
    if let Some(parent) = base.parent() {
        parent.base().child_devices.lock().push(Arc::downgrade(this));
    }
    Ok(())
}
```

Every `?` in that function unwinds through `teardown`, which is what makes
registration atomic (R14). Three details are worth reading closely:

- **Step 2 and 3 build one list and add it once.** `AttrTable::add` is
  all-or-nothing, so a duplicate name in any layer fails the whole registration
  rather than leaving a partial directory.
- **Step 5 stores the node request.** The `/dev` node is computed once here, and
  the `uevent` variables read that stored value, so the file user space sees and
  the variables it is told cannot disagree even if a `devnode` callback is not a
  pure function of the device.
- **Step 7 is where a driver's `probe` runs**, inside the parent's `add`, which is
  what the `Adding` state exists for.

### `remove` and `teardown`

```rust
/// Unregisters a device: the counterpart of Linux's `device_del`.
///
/// Unlike Linux, a device with registered child devices cannot be removed;
/// remove the children first. A bound bus device is unbound on the way, but
/// since a driver's `remove` is usually what deletes the children it created,
/// the caller's rule is: unbind first, then remove.
pub fn remove<D: AnyDevice + ?Sized>(dev: &Arc<D>) -> Result<()> {
    let this = dev.to_arc();
    let base = this.base();
    {
        let mut state = base.state.lock();
        if *state != State::Added {
            return Err(Error::NotAdded);
        }
        if !base.child_devices().is_empty() {
            return Err(Error::HasChildren);
        }
        *state = State::Removed;
    }
    teardown(&this, Teardown::Announced);
    Ok(())
}
```

`teardown` serves both removal and the failure path of `add`, so every step is
written to tolerate a step that never happened. The `Teardown` argument says
whether user space ever saw the device:

```rust
/// Whether `teardown` announces the removal to user space.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Teardown {
    /// A registered device is being removed: send the `remove` uevent.
    Announced,
    /// A registration is being undone: user space never saw the device.
    Silent,
}

/// Undoes registration, tolerating steps that never happened.
fn teardown(this: &Arc<dyn AnyDevice>, mode: Teardown) {
    let base = this.base();
    let subsystem = this.subsystem();
    // Only links this device created are removed: an index entry under the
    // same name may belong to another device whose name this one clashed with.
    let links = core::mem::take(&mut *base.links.lock());

    // 8. Unlink from the parent.
    if let Some(parent) = base.parent() {
        parent
            .base()
            .child_devices
            .lock()
            .retain(|w| w.upgrade().is_some_and(|d| !Arc::ptr_eq(&d, this)));
    }
    // 7. Let the subsystem act.
    if let Some(ops) = subsystem.ops() {
        ops.on_removed(this);
    }
    // 6. Announce.
    if mode == Teardown::Announced {
        emit_uevent(this.as_ref(), UeventAction::Remove);
    }
    // 5. Device number.
    if let Some(devnum) = base.devnum() {
        // Taken in its own statement: the guard of an `if let` scrutinee lives
        // to the end of the block, which would hold this device's `devnode`
        // mutex across the hook and put the hook queue underneath it.
        let spec = base.devnode.lock().take();
        if let Some(spec) = spec {
            let _ = hooks::delete_devnode(&spec.request);
        }
        if links.dev_index {
            remove_link(registry().dev_index(devnum.kind()).as_ref(), &devnum.to_string());
        }
    }
    // 4. Links.
    if links.index
        && let Some(index) = subsystem.index_dir()
    {
        remove_link(index.as_ref(), base.name());
    }
    if links.device {
        remove_link(base, "device");
    }
    if links.subsystem {
        remove_link(base, "subsystem");
    }
    // 3. and 2. Attributes are dropped with the directory.
    // 1. Detach the directory, and any glue directory left empty.
    let Some(tree_parent) = base.tree_parent.get() else {
        return;
    };
    tree_parent.with_edit(|parent| {
        let _ = parent.detach_child(base.name());
    });
    let class = subsystem.name().unwrap_or_default();
    match (subsystem.kind(), base.parent()) {
        (SubsystemKind::Class, Some(parent)) if uses_glue_dir(&subsystem, parent) => {
            parent.base().glue_dirs.drop_if_empty(class, parent.base());
        }
        (SubsystemKind::Class, None) => registry().drop_virtual_glue_dir_if_empty(class),
        _ => {}
    }
}
```

The deletion of the `/dev` node uses the *stored* request, not a fresh computation,
so the node deleted is the node created even if the class's `devnode` callback
would now answer differently.

### How the steps map onto Linux's `device_add`

| Linux `device_add` step | Here |
|---|---|
| 1. Fix the name (a bus-supplied prefix plus an id) | Given to the builder; validated in `add`. |
| 2. Choose the parent directory; create `virtual` or a glue directory | Step 1, `place`. |
| 3. Create the device directory | Step 1, the attach inside `place`. |
| 4. Create the `uevent` file | Step 2. |
| 5. Class symlinks: `subsystem`, `device`, `/sys/class/<c>/<name>` | Step 4. |
| 6. Attribute groups of class, type, and device | Step 3. |
| 7. Bus: attributes, `/sys/bus/<b>/devices/<name>`, `subsystem`, list | Steps 3, 4, 7; no `driver_override`. |
| 8. `dpm_sysfs_add`, which creates `power/` | Not modeled; see the limitations. |
| 9. `dev` attribute, `/sys/dev` link, devtmpfs node | Steps 2 and 5. |
| 10. `add` uevent | Step 6. |
| 11. `bus_probe_device` | Step 7, the bus's `on_added`. |
| 12. Parent's child list, class list, class interfaces | Steps 7 and 8. |

Three orderings differ from Linux. Two of them make no observable difference. Linux
creates class symlinks before attribute groups; here all attributes come first, and
no callback runs in between. Linux links the device into its parent's child list
before it touches the class list; here the subsystem comes first and the parent
last.

The third does make a difference, in a narrow window. Linux fills the bus's own
device list in `bus_add_device`, before the `add` uevent; here step 7 fills it
after. So a write to `bind`, `unbind` or `drivers_probe` naming a device whose `add`
is still in flight fails with `ENOENT`, where Linux would already find it. The
device's directory and its `/sys/bus/<b>/devices/<name>` link exist by then, from
steps 1 and 4, so user space can see the device it cannot yet name. Nothing in the
kernel depends on that window, and closing it would mean filling the list before the
subsystem callback that owns it runs.

## Drivers and binding

```rust
/// A driver for devices on bus `B`.
pub trait Driver<B: Bus>: Send + Sync + 'static {
    /// The driver name: the directory under `/sys/bus/<bus>/drivers`.
    fn name(&self) -> &str;

    /// The devices this driver accepts, in the bus's terms.
    fn match_data(&self) -> &B::MatchData;

    /// Takes over a matched device. Return an error to decline it.
    fn probe(&self, dev: &Arc<BusDevice<B>>) -> Result<()>;

    /// Releases a device before it is unbound or removed.
    fn remove(&self, _dev: &Arc<BusDevice<B>>) {}

    /// Attributes every bound device gets while it is bound.
    fn dev_attrs(&self) -> &'static [Attr<BusDevice<B>>] { &[] }
}
```

The type parameter is what makes a driver belong to a bus: `register_driver` is a
method on `BusHandle<B>` and takes an `Arc<dyn Driver<B>>`, so a driver written for
one bus cannot be registered on another.

It returns an `Arc<DriverHandle<B>>`, the third of the runtime-state handles. A
`DriverHandle<B>` holds the driver itself, its directory under
`/sys/bus/<bus>/drivers`, the list of devices currently bound to it, and a flag
that `unregister_driver` clears so that a bind racing with it fails. It
dereferences to `dyn Driver<B>`, which is why the code below calls
`driver.match_data()`, `driver.probe(dev)` and `driver.dev_attrs()` straight on the
handle: those are the driver's own methods.

One helper appears in the code below that has not been named yet:
`remove_driver_links(dev, driver)` deletes the pair of symlinks that binding
created, `<device>/driver` and `<driver>/<device>`.

Binding is symmetric (R11): `register_driver` offers the new driver every unbound
device, and `on_added` offers each new device to every driver. Both offers are
gated on the bus's `autoprobe` flag, which is what the `drivers_autoprobe` control
file switches; with it off, binding happens only when user space asks. Both reach
`try_bind`:

```rust
    fn try_bind(&self, dev: &Arc<BusDevice<B>>, driver: &Arc<DriverHandle<B>>) -> Result<()> {
        if !self.bus.matches(dev.payload(), driver.match_data()) {
            return Err(Error::NoDriver);
        }
        let _guard = dev.bind_lock().lock();
        if !dev.base().is_added() {
            return Err(Error::NotAdded);
        }
        if !driver.is_registered.load(Ordering::Relaxed) {
            return Err(Error::DriverUnregistered);
        }
        if dev.driver().is_some() {
            return Err(Error::AlreadyBound);
        }
        // The links come first so that `probe` sees the device as Linux's
        // drivers do; they are removed again if `probe` declines.
        add_link(driver.dir.as_ref(), dev.base().name(), &dev.base().tree_path())?;
        if let Err(e) = add_link(dev.base(), "driver", &SysObj::path(driver.dir.as_ref())) {
            remove_link(driver.dir.as_ref(), dev.base().name());
            return Err(e);
        }
        if let Err(e) = driver.probe(dev) {
            remove_driver_links(dev, driver);
            return Err(match e {
                Error::NoDriver => Error::NoDriver,
                _ => Error::ProbeFailed,
            });
        }
        if let Err(e) = dev.add_attrs(driver.dev_attrs()) {
            driver.remove(dev);
            remove_driver_links(dev, driver);
            return Err(e);
        }
        dev.set_driver(driver.clone());
        driver.bound.lock().push(dev.clone());
        Ok(())
    }
```

Reading it against Linux's `really_probe`: the two links exist before `probe`, so a
driver can create children that reference them; the driver's attributes are added
only after `probe` succeeds; a declining driver leaves no trace, and the caller
tries the next one. The two error kinds are kept apart because they mean different
things to a caller: `NoDriver` is "not mine", `ProbeFailed` is "mine, and it went
wrong".

Unbinding runs the same steps backwards, in Linux's order:

```rust
    fn unbind_inner(
        &self,
        dev: &Arc<BusDevice<B>>,
        expected: Option<&Arc<DriverHandle<B>>>,
    ) -> Result<()> {
        let _guard = dev.bind_lock().lock();
        if let Some(expected) = expected
            && !dev.driver().is_some_and(|d| Arc::ptr_eq(&d, expected))
        {
            return Err(Error::NotBound);
        }
        let driver = dev.driver().ok_or(Error::NotBound)?;
        // The links go first, then the driver's attribute files, then its
        // `remove`, which is the order of Linux's `__device_release_driver`,
        // so that nothing user space can open outlives the driver's hold on
        // the device.
        remove_driver_links(dev, &driver);
        dev.remove_attrs(driver.dev_attrs());
        driver.remove(dev);
        dev.take_driver();
        driver.bound.lock().retain(|d| !Arc::ptr_eq(d, dev));
        Ok(())
    }
```

The `expected` parameter exists for the sysfs `unbind` file: a write names a driver
as well as a device, and the check that the named driver is still the bound one has
to happen under the same lock as the unbind, or a concurrent rebind can make the
write tear down a driver the writer never named.

### The control files

`BusDirOps` and `DriverDirOps` serve the four files of R12. Two behaviors are
copied from Linux rather than invented:

```rust
            "drivers_probe" => {
                let dev = bus.find_device(value.trim()).ok_or(Error::NotFound)?;
                // Linux's `drivers_probe_store` reports success when the
                // device is already bound or when no driver matches.
                match bus.probe(&dev) {
                    Ok(()) | Err(Error::AlreadyBound) | Err(Error::NoDriver) => {}
                    Err(e) => return Err(e),
                }
            }
```

and the `unbind` file routes to `unbind_from`, the identity-checking form:

```rust
impl<B: Bus> DirAttrOps for DriverDirOps<B> {
    fn store(&self, name: &str, value: &str) -> Result<()> {
        let driver = self.driver.upgrade().ok_or(Error::NotFound)?;
        let bus = driver.bus().ok_or(Error::NotFound)?;
        let dev = bus.find_device(value.trim()).ok_or(Error::NotFound)?;
        match name {
            "bind" => bus.bind(&dev, &driver),
            "unbind" => bus.unbind_from(&dev, &driver),
            _ => Err(Error::NotFound),
        }
    }
}
```

Unregistering a driver has to leave nothing bound to it. It clears a flag first, so
that no *new* bind starts, then unbinds every device that might hold it:

```rust
    /// Unregisters a driver, unbinding its devices first.
    pub fn unregister_driver(&self, driver: &Arc<DriverHandle<B>>) -> Result<()> {
        let removed = { /* remove from self.drivers, error if absent */ };
        // No new bind starts from here on, and any bind already in flight
        // finishes before `unbind_from` takes the same device's binding lock.
        driver.is_registered.store(false, Ordering::Relaxed);
        for dev in self.devices() {
            let _ = self.unbind_from(&dev, driver);
        }
        // A device that is being removed has already left the bus list but may
        // not be unbound yet, so the driver's own list is drained as well.
        for dev in driver.devices() {
            let _ = self.unbind_from(&dev, driver);
        }
        let _ = self.drivers_dir.detach_child(driver.name());
        Ok(())
    }
```

The two passes are not redundant. The first covers a bind that is in flight, whose
device is already on the bus list but not yet on the driver's; the second covers a
device that has left the bus list because it is being removed but is not unbound
yet.

## Device numbers, `/dev` nodes, and uevents

### Numbers

```rust
/// Whether a device number names a character or a block device.
pub enum DevKind { Char, Block }

/// A device number together with its kind.
pub struct DevNum {
    kind: DevKind,
    id: DeviceId,
}

impl fmt::Display for DevNum {
    /// Formats as `major:minor`, the form used by `/sys/dev` and the `dev`
    /// attribute.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.id.major().get(), self.id.minor().get())
    }
}
```

Linux decides char versus block by comparing the device's class against
`block_class`; carrying the kind in the number is the Rust substitute for that
pointer comparison, and it is what selects `/sys/dev/char` or `/sys/dev/block` and
the file type of the `/dev` node.

### The node request

```rust
/// A request to create or delete a `/dev` node.
pub struct DevNodeRequest {
    /// The device number the node refers to.
    pub devnum: DevNum,
    /// The path of the node relative to `/dev`, e.g. `null` or `input/event0`.
    pub path: SysStr,
    /// The permission bits of the node.
    pub mode: u16,
}

/// The default mode of a device node when neither the device type nor the
/// class overrides it (Linux devtmpfs uses `0600` as well).
pub const DEFAULT_DEVNODE_MODE: u16 = 0o600;

/// What a `devnode` hook may override for a device's `/dev` node.
pub struct DevNode {
    /// The node path relative to `/dev`; `None` keeps the device name.
    pub path: Option<SysStr>,
    /// The permission bits; `None` keeps the default.
    pub mode: Option<u16>,
}
```

What registration stores is the request plus one bit of provenance, because Linux
prints `DEVMODE` only when a callback chose the mode rather than defaulting it
(`dev_uevent`):

```rust
/// The `/dev` node a device was registered with, and whether a `devnode`
/// callback chose its mode (Linux reports `DEVMODE` only if one did).
pub(crate) struct DevNodeSpec {
    pub(crate) request: DevNodeRequest,
    pub(crate) has_mode: bool,
}
```

The request is computed once, in registration step 5:

```rust
fn devnode_spec(dev: &dyn AnyDevice, devnum: DevNum) -> DevNodeSpec {
    let over = dev.devnode_override();
    let has_mode = over.as_ref().is_some_and(|o| o.mode.is_some());
    let over = over.unwrap_or_default();
    DevNodeSpec {
        request: DevNodeRequest {
            devnum,
            path: over
                .path
                .unwrap_or_else(|| SysStr::from(dev.base().name().replace('!', "/"))),
            mode: over.mode.unwrap_or(DEFAULT_DEVNODE_MODE),
        },
        has_mode,
    }
}
```

Two Linux rules are in those few lines. The `!`-to-`/` substitution turns a device
whose own name embeds a path separator as `!`, historically a block name such as
`cciss!c0d0`, into the node `cciss/c0d0`, which is why the hook's contract requires
it to create intermediate directories. It is the fallback branch only: a node like
`input/event0` comes from the input class's `devnode` callback prefixing `input/`,
not from the device's name. And `has_mode` records whether a callback chose the
mode, because Linux prints `DEVMODE` only when one did.

For a class device the two callbacks compose the way `device_get_devnode` composes
them: the type is asked first, and the class only if the type named no node.

```rust
    fn devnode_override(&self) -> Option<DevNode> {
        // As in Linux's `device_get_devnode`: the type is asked first, and the
        // class only if the type named no node. A type that sets just a mode
        // still lets the class name the node, and the class's mode takes
        // precedence if it sets one (Linux passes one `mode` pointer to both
        // callbacks, so whichever writes last wins; here the `or` says it).
        let from_type = self.declared.type_devnode(self);
        if from_type.as_ref().is_some_and(|node| node.path.is_some()) {
            return from_type;
        }
        let from_class = self.class.class().devnode(self);
        match (from_type, from_class) {
            (None, class) => class,
            (Some(t), None) => Some(t),
            (Some(t), Some(c)) => Some(DevNode {
                path: c.path,
                mode: c.mode.or(t.mode),
            }),
        }
    }
```

### Uevents

```rust
/// What happened to a device.
pub enum UeventAction { Add, Remove, Change, Bind, Unbind }

/// The `KEY=VALUE` variables of a uevent, in insertion order.
pub struct UeventVars { vars: Vec<(String, String)> }

/// A complete uevent, ready to be broadcast.
///
/// The fields are private so that the sequence number can only come from the
/// counter in `Uevent::new`.
pub struct Uevent {
    action: UeventAction,
    devpath: String,
    subsystem: String,
    vars: UeventVars,
    seqnum: u64,
}
```

The device variables are built in one place, and this is the function that decides
what user space reads from the `uevent` file and receives in an event:

```rust
/// Builds the `uevent` attribute content: the device-specific variables.
fn dev_uevent_vars(dev: &dyn AnyDevice) -> UeventVars {
    let mut vars = UeventVars::new();
    if let Some(devnum) = dev.base().devnum() {
        vars.add("MAJOR", devnum.id().major().get());
        vars.add("MINOR", devnum.id().minor().get());
        // The node the device was registered with, so that the variables and
        // the file in `/dev` agree even if a `devnode` callback is not pure.
        // Before that node exists, the callbacks are asked again.
        let node = dev
            .base()
            .devnode_spec()
            .unwrap_or_else(|| devnode_spec(dev, devnum));
        vars.add("DEVNAME", &node.request.path);
        // Linux prints the mode only when a `devnode` callback set one, with
        // `%#03o` over the permission bits, e.g. `0666`.
        if node.has_mode {
            vars.add("DEVMODE", format_args!("0{:o}", node.request.mode & 0o777));
        }
        // (Linux writes the same thing with `%#03o`, which agrees with this
        // for every three-digit mode, and skips a mode of zero
        // because it cannot tell "unset" from "0"; the `has_mode` flag here
        // makes that distinction, at the cost of printing `DEVMODE=00` for a
        // callback that deliberately asks for mode zero, where Linux prints
        // nothing.)
    }
    if let Some(type_name) = dev.type_name() {
        vars.add("DEVTYPE", type_name);
    }
    if let Some(driver) = dev.driver_name() {
        vars.add("DRIVER", driver);
    }
    dev.subsystem_uevent(&mut vars);
    vars
}
```

The order is Linux's: the core variables, then the subsystem's, then the type's.
The last two both come out of the single `dev.subsystem_uevent` call above, because
each device struct implements it as "the bus's or class's callback, then the
type's".

A device with no subsystem sends no event at all, which is what Linux's
`dev_uevent_filter` does, so bare devices and glue directories stay silent:

```rust
fn emit_uevent(dev: &dyn AnyDevice, action: UeventAction) {
    let Some(subsystem) = dev.subsystem().name().map(String::from) else {
        // Devices without a subsystem send no events, as in Linux.
        return;
    };
    let event = Uevent::new(action, dev.base().tree_path(), subsystem, dev_uevent_vars(dev));
    hooks::broadcast_uevent(event);
}
```

### Reaching devtmpfs and the socket

A component may not depend on the kernel crate (R16), so the dependency is
inverted: the model declares what it needs, and the kernel crate provides it.

```rust
/// What the kernel crate provides to the device model.
pub trait KernelHooks: Send + Sync + 'static {
    /// Creates a `/dev` node.
    ///
    /// A path with `/` in it, such as `input/event0`, means the intermediate
    /// directories are created too.
    fn create_devnode(&self, request: &DevNodeRequest) -> Result<(), HookError>;

    /// Deletes a `/dev` node created earlier. The request is the one that
    /// created the node.
    fn delete_devnode(&self, request: &DevNodeRequest) -> Result<(), HookError>;

    /// Delivers a uevent to user space.
    fn broadcast_uevent(&self, event: &Uevent);
}
```

`HookError` is a unit struct: the device model has nothing useful to do with a
reason, so the hooks report only that the call failed.

The hooks cannot be installed at component-init time, because devtmpfs is not
running yet; the kernel crate installs them at the start of the device subsystem's
first-kernel-thread initialization, which runs after the file-system init that
spawns `devtmpfsd`. Devices registered before that are not an error: their requests
are queued and replayed.

```rust
/// A request made before the hooks were installed: either a `/dev` node to
/// create or an event to deliver.
enum Pending {
    Create(DevNodeRequest),
    Event(Uevent),
}

/// The hooks, once installed, and the requests waiting for them.
///
/// A slot exists so that the queue-and-replay behavior can be tested without
/// touching the one the kernel installs into.
pub(crate) struct HookSlot {
    hooks: Once<Arc<dyn KernelHooks>>,
    pending: Mutex<Vec<Pending>>,
}

impl HookSlot {
    /// Installs the hooks and replays every request queued before.
    ///
    /// Installing, draining and replaying all happen under the queue lock, so
    /// a request made concurrently is either replayed here or delivered
    /// directly, and a removal cannot overtake the create it cancels.
    /// Calling this a second time has no effect.
    pub(crate) fn install(&self, hooks: Arc<dyn KernelHooks>) {
        let mut pending = self.pending.lock();
        self.hooks.call_once(|| hooks);
        let installed = self.hooks.get().expect("the hooks were just installed here");
        for item in core::mem::take(&mut *pending) {
            match item {
                // A failure here cannot be reported to the caller that queued
                // the request long ago; the node is simply absent.
                Pending::Create(request) => { let _ = installed.create_devnode(&request); }
                Pending::Event(event) => installed.broadcast_uevent(&event),
            }
        }
    }

    /// Returns the installed hooks, or queues `pending` if there are none yet.
    fn hooks_or_queue(&self, pending: impl FnOnce() -> Pending) -> Option<&Arc<dyn KernelHooks>> {
        let mut queue = self.pending.lock();
        match self.hooks.get() {
            Some(hooks) => Some(hooks),
            None => {
                queue.push(pending());
                None
            }
        }
    }
}
```

Installation, draining and replay all happen under the queue lock, so a device
registered and removed across the installation cannot leave an orphaned node. Once
the hooks are in place the lock is released before each call, so the calls
themselves are not serialized.

The kernel-side implementation is small. Node creation maps onto devtmpfs; event
delivery is where the netlink socket will be wired in, and today logs:

```rust
impl KernelHooks for Hooks {
    fn create_devnode(&self, request: &DevNodeRequest) -> core::result::Result<(), HookError> {
        let node = to_devtmpfs_node(request).map_err(|_| HookError)?;
        devtmpfs::create_node(node).map_err(|error| {
            warn!("failed to create devtmpfs node {:?}: {:?}", request.path, error);
            HookError
        })
    }

    // `delete_devnode` is the mirror image and is elided here.

    fn broadcast_uevent(&self, event: &Uevent) {
        // TODO: Deliver the event through the `NETLINK_KOBJECT_UEVENT` socket
        // family; the multicast path exists but is not yet wired up.
        debug!(
            "uevent: {} {} (subsystem {}, seq {})",
            event.action(), event.devpath(), event.subsystem(), event.seqnum()
        );
    }
}
```

## Concurrency and lifetimes

Two things must be settled for a registry of shared objects: what keeps a device
alive, and which operations may run at once.

### Ownership

Every handle is an `Arc`. A device holds its parent, its subsystem handle and its
driver strongly. Its parent holds it weakly in `child_devices` and strongly through
the tree, so detaching the directory is what makes a removed device collectable.
The bus and class handles hold their members strongly while they are registered,
which is a cycle broken by `on_removed`. A bound driver's handle likewise holds the
device strongly in its list of bound devices, a second cycle, broken by `unbind`.
Registered buses and classes are kept alive for the life of the kernel by the
registry, so dropping the handle a `register_*` call returned loses nothing — and,
as a consequence, a bus or a class cannot be unregistered at all. Linux can
(`bus_unregister`, `class_unregister`), because its subsystems can live in modules;
nothing in Asterinas needs it yet, and the restriction is listed among the
limitations.

### The locks

Five locks carry design decisions:

- **The registration state**, a mutex-protected enum per device. It serializes
  `add` and `remove` for that one device, and `remove` holds it across the
  `HasChildren` check, so a child whose `add` has *completed* cannot be missed.
- **A binding lock per bus device**, Linux's `dev->mutex`, held across `try_bind`
  and `unbind`. A `probe` that adds a child device takes, underneath it, everything
  `add` takes: the parent's glue directories, and the child's class membership and
  device lists.
- **The glue directories of a parent**, one lock held across "find or create, then
  attach" and across "check empty, then detach" (Linux's `gdp_mutex`).
- **A membership lock per class**, Linux's `sp->mutex`, held while a member joins
  or leaves and while an interface registers, so that an interface sees each member
  exactly once.
- **The device and driver lists** on the bus and class handles.

What the state lock does *not* buy is worth stating on its own. It does not make
"remove a parent while a child is being added" safe. `add` releases the lock before
running the eight steps, which is what lets a driver's `probe` register children,
and a child joins its parent's child list only in the last of those steps. So not
removing a parent concurrently with a child's `add` is a caller obligation, stated
again at the end of this subsection.

Six more are plumbing, and are named here because an implementer meets them:

- **A class's interface list**, a mutex separate from the membership lock and
  always taken under it, so that a registering interface and a joining device
  cannot interleave.
- **A bus device's bound driver**, a read-write lock read by every `driver_name`
  and written only under that device's binding lock.
- **A device's attribute table**, a read-write lock over both the published
  attribute set and the callbacks behind it, so the two cannot disagree. The
  callbacks are cloned out of it and run after it is released.
- **A device's `children` map**, another read-write lock, which is what a tree walk
  reads.
- **The hook queue.** Nothing else is taken under it, except during `install`'s
  replay, which runs the queued hooks with it held.
- **Three short mutexes on a device** (`links`, `devnode`, `child_devices`) that are
  never held across a callback or a hook. `devnode` is the one that has to be
  watched: `teardown` deliberately takes and drops it in a statement of its own,
  because leaving it in an `if let` scrutinee would hold it across the `/dev`-node
  hook and put the hook queue underneath it.

The order, when more than one is held, is: binding lock, then a device's
registration state, then class membership, then a class's interface list, then the
subsystem lists, then glue directories, then a device's children, then the
attribute table and the attribute set under it, with the hook queue below all of
them. No path in the crate takes them in another order, and the two rules below are
what keeps callbacks from creating one.

Callbacks run under two of them, which gives two rules, both of them Linux's as
well:

1. A driver's `probe` and `remove` run under the binding lock, so neither may
   remove or unbind that device. A driver removes the children it created; the
   caller then removes the device.
2. A class interface's callbacks run under the membership lock, so neither may
   call `add`, `remove`, `bind` or `unbind`, directly or through a driver it
   triggers. The binding lock is always taken before the membership lock, never the
   other way round.

Two failure modes follow from that arrangement. A registered device that is simply
dropped without `remove` does not disappear: the directory it was placed in holds a
strong reference — its parent's children map, or the glue directory under
`/sys/devices/virtual` for a parentless class device — so it stays in `/sys` and the
`Arc` is never freed.
Registration is what publishes a device, and only `remove` unpublishes it. And
`teardown` ignores the result of every step it takes, because there is no caller
left to report to and a half-undone registration is worse than a best-effort one;
the steps are written so that each can fail independently without invalidating the
others.

Callers must not remove a parent while adding a child. Linux instead pins the
parent by taking a reference to it (`get_device`) for the child's lifetime; here
the parent's child list and the `HasChildren` error that `remove` returns serve the
same purpose once the child's `add` has completed.

## Converting an existing driver

The `mem` conversion below is the worked example; this is the checklist it follows,
and the one the block, input and tty conversions will follow.

1. **Decide what the device is.** If the hardware is enumerated by something, it is
   a `BusDevice<B>` on that bus. If it is the face a driver presents to user space,
   it is a `ClassDevice<C>`. If it is neither and exists to be a parent, it is a
   `BareDevice`. A driver that today creates one object serving both roles becomes
   a bus device plus a class device the driver creates in `probe`.
2. **Choose the payload.** `B::Device` or `C::Device` is whatever the driver
   already keeps per device; the conversion usually makes an existing struct the
   payload rather than introducing a new one.
3. **Move naming policy into the class.** A `devnode` callback replaces whatever
   ad hoc `/dev` naming the driver did, and the model creates and deletes the node.
4. **Move per-device files into attributes.** Anything the driver would have
   printed into a private sysfs node becomes an `Attr<D>` on the bus, the class,
   the device type, or the device.
5. **Register through `add`, and keep the number registry.** The device model
   answers "what exists"; the char or block registry still answers "what does this
   number open". Register with the model first, then with the registry, and undo
   the first if the second fails.
6. **Delete the driver's own removal code.** `remove` undoes everything `add` did.

## Checking the design against the requirements

| Requirement | Where it is met |
|---|---|
| R1 one device tree | `DeviceBase::parent` and `name`; the `/sys/devices` root created at initialization |
| R2 one subsystem per device | three device structs; `Subsystem` follows from the struct, so the combination is unrepresentable |
| R3 placement rules | `place`; the bus-supplied root is a gap |
| R4 glue directories | `GlueDirs`, created on first use and dropped when empty |
| R5 device types | `DeviceType<D>`, a `static` a builder points at |
| R6 three indexes | registration steps 4 and 5 |
| R7 back links | registration step 4, `DeviceType::has_device_link`, and `try_bind` for the driver links |
| R8 layered attributes | `AttrTable` with the core, subsystem, type, own and driver layers |
| R9 device numbers and `/dev` | `DevNum`, the `dev` attribute, `devnode_spec` and the request it stores, `KernelHooks` |
| R10 uevents | `Uevent` and the `uevent` attribute; `add` and `remove` only, and delivery is stubbed |
| R11 buses match and bind | `try_bind`, called from `register_driver` and from the bus's `on_added` |
| R12 user-space control | `BusDirOps` and `DriverDirOps` behind the four files |
| R13 class interfaces | `ClassHandle::register_interface`, with replay for existing members |
| R14 atomic registration | `teardown` on every failure path; the `Adding` and `Removed` states |
| R15 hot removal is visible | the sysfs revalidation hooks |
| R16 component boundaries | `KernelHooks` with the pending queue |
| R17 invalid states unrepresentable | the type parameters on the relationships: three device structs, `Driver<B>`, `Attr<D>` |
| R18 do not fork `systree` | two changes in `systree`, one in sysfs; the shared inode layer absorbs them, so cgroupfs and configfs are untouched |

## Putting it together: `/dev/null`

The `mem` character devices are the design's first real conversion, and they
exercise most of it: a class, a class device with no parent, a device number, a
`devnode` policy, a devtmpfs node, and a second registry that still has to work.

In Linux these five devices — `null`, `zero`, `full`, `random`, `urandom` — are
class devices of the `mem` class with major number 1 and no parent, created by
`device_create` (`drivers/char/mem.c`). In Asterinas before this work they were
char-registry entries that created their own devtmpfs nodes.

**The class.** The payload is the existing `MemFile` enum, so the object in the
sysfs tree is the object that knows how to serve reads and writes:

```rust
/// The `mem` class: memory devices such as `/dev/null`.
pub(crate) struct MemClass;

impl Class for MemClass {
    const NAME: &'static str = "mem";
    type Device = MemFile;

    fn devnode(&self, dev: &ClassDevice<Self>) -> Option<DevNode> {
        // Linux's memory-device table uses nonzero modes only for devices
        // that override devtmpfs's default permissions.
        let mode = match **dev {
            MemFile::Full | MemFile::Null | MemFile::Random | MemFile::Urandom | MemFile::Zero => {
                mkmod!(a+rw)
            }
            MemFile::Kmsg => mkmod!(a+r, u+w),
            _ => return None,
        };
        Some(DevNode { path: None, mode: Some(mode.bits()) })
    }
}

/// A memory device, as seen by the char registry.
pub(crate) type MemDevice = ClassDevice<MemClass>;
```

`match **dev` is the `Deref` at work: one `*` for the `&ClassDevice<MemClass>`
reference and one for the `ClassDevice<MemClass> -> MemFile` `Deref`.

**The second registry.** Asterinas keeps a map from device number to something
openable, which is Linux's `cdev` map and stays as it is. What changes is that the
same object serves both roles, and that the map no longer creates the `/dev` node:

```rust
// Note: this `DeviceType` is the kernel crate's char-or-block enum, not the
// device model's `DeviceType<D>` of the previous sections.
impl Device for MemDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        self.base()
            .devnum()
            .expect("memory devices always have a device number")
            .id()
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsNodeMeta> {
        // The device model creates the node when the device is added.
        None
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        Ok(Box::new(**self))
    }
}
```

**Registration.** One builder call and two registrations, with the second's failure
undoing the first:

```rust
fn add_device(file: MemFile) -> Result<()> {
    let class = MEM_CLASS.get().unwrap();
    let id = DeviceId::new(MEM_MAJOR.get().unwrap().get(), MinorId::new(file.minor()));
    let device = MemDevice::builder(class, file.name(), file)
        .devnum(DevNum::char(id))
        .build();
    aster_device::add(&device)?;
    // The number-to-open map is the second registry; if it refuses the
    // device, the first registration is undone rather than left dangling.
    if let Err(error) = register(device.clone()) {
        let _ = aster_device::remove(&device);
        return Err(error);
    }
    Ok(())
}

pub(super) fn init_in_first_kthread() {
    MEM_MAJOR.call_once(|| acquire_major(MajorId::new(1)).unwrap());
    MEM_CLASS.call_once(|| aster_device::register_class(MemClass).unwrap());

    add_device(MemFile::Full).unwrap();
    add_device(MemFile::Null).unwrap();
    add_device(MemFile::Random).unwrap();
    add_device(MemFile::Urandom).unwrap();
    add_device(MemFile::Zero).unwrap();
}
```

Five names across the listings above come from the kernel crate rather than the
model. `mkmod!` builds mode bits from a symbolic spec, so `mkmod!(a+rw)` is `0666`.
`acquire_major` reserves a major number in the char registry and returns a
`MajorIdOwner` that holds it. `register` puts the device in that registry's
number-to-open map, `PerOpenFileOps` is what an `open()` on the resulting `/dev`
node returns, and `DevtmpfsNodeMeta` is how a device used to ask that registry to
create its node — returning `None` is how this conversion says the model owns that
now. Registration happens in the first kernel thread because that is where
the kernel already initializes its devices, immediately after installing the hooks,
once the file-system init has spawned `devtmpfsd`; the model itself would tolerate
an earlier call, since requests made before the hooks arrive are queued.

**What the kernel produces.** Booted under QEMU with the test initramfs, walking
`/sys` gives, for `null`, in the notation the Overview's listing introduced:

```text
/sys/devices/virtual/mem/null/
/sys/devices/virtual/mem/null/dev        = 1:3
/sys/devices/virtual/mem/null/uevent     = MAJOR=1 MINOR=3 DEVNAME=null DEVMODE=0666
/sys/devices/virtual/mem/null/subsystem -> ../../../../class/mem
/sys/class/mem/null                     -> ../../devices/virtual/mem/null
/sys/dev/char/1:3                       -> ../../devices/virtual/mem/null
```

and the same shape for `zero` (1:5), `full` (1:7), `random` (1:8) and `urandom`
(1:9), plus the five nodes in `/dev`:

```text
crw-rw-rw-    1 root     root        1,   3 /dev/null
```

A Linux v7.2 guest on the same QEMU machine prints those six lines for each of the
five devices, differing only in the name and the minor number, plus the `power/`
directory and the five attributes inside it — `autosuspend_delay_ms`, `control`,
`runtime_active_time`, `runtime_status` and `runtime_suspended_time`, counted from
the unpruned capture, since the pruned tree drops `power/` entirely — which this
model does not have.

Every artifact in that listing came from a different part of the design, and this
is the shortest way to see the whole thing at work:

| Line | Produced by |
|---|---|
| the directory under `virtual/mem/` | `place`, the parentless class-device case, creating the glue directory |
| `dev` | registration step 2, the core `DEV_ATTR` |
| `uevent` | registration step 2, the core `uevent` attribute, filled by `dev_uevent_vars` |
| `DEVMODE=0666` | `MemClass::devnode`, recorded in the stored `DevNodeSpec` |
| `subsystem ->` | registration step 4 |
| `/sys/class/mem/null ->` | registration step 4, the index link |
| `/sys/dev/char/1:3 ->` | registration step 5 |
| `/dev/null` | registration step 5, through `KernelHooks::create_devnode` |

# Limitations, Discussions, and Future Work

## What the prototype implements, and how it was checked

The prototype is five commits on top of `ab9a4cfdc`, on the branch
[`device-model`](https://github.com/tatetian/asterinas/tree/device-model):

| Commit | What it does |
|---|---|
| [`b7a4b2992`](https://github.com/tatetian/asterinas/commit/b7a4b299246cf6e86142cc3cbdc569dc936b0c30) | `systree`: a node may replace its attribute set, and `relative_path` |
| [`4ad6aae04`](https://github.com/tatetian/asterinas/commit/4ad6aae04adc761fb3afb20618831bb3adb2e656) | sysfs: revalidate cached dentries against the live tree |
| [`7e9a1d4c7`](https://github.com/tatetian/asterinas/commit/7e9a1d4c7b101804b2f9f4291cf5b0022229e029) | the `aster-device` component itself, with its kernel-mode tests |
| [`226e50c71`](https://github.com/tatetian/asterinas/commit/226e50c71da2dbc50d8fe2a1b3e92c742e46a5c8) | the `mem` conversion and the kernel-side hooks |
| the branch tip | this document |

What that adds up to:

- **Real conversion.** The five `mem` devices register through the model, and the
  six lines each of them produces are the ones printed in the `/dev/null`
  walkthrough above. Booting the kernel under QEMU with the test initramfs and
  walking `/sys` produces, for those devices, output identical to a Linux v7.2
  guest on the same machine, line for line, apart from the `power/` directory Linux
  adds to every device and two artifacts of how the Linux tree was captured.
- **What is not on the model yet.** `/sys/bus` is empty: no real bus is converted,
  so buses and drivers exist only in the kernel-mode tests. Every other device
  Asterinas has — tty, input, block, framebuffer, hardware RNG — still registers
  through the old path and does not appear under `/sys/devices`.
- **A second, smaller conversion.** The TSM measurement node no longer invents
  `/sys/devices/virtual/misc`; it asks the model for that directory. It is not yet
  a class device, and it is the only caller of the escape hatch.
- **Buses, drivers, classes and interfaces** are exercised by eleven kernel-mode
  tests against a synthetic bus, covering match, probe, bind, unbind, removal, the
  control files (`bind`, `unbind` and `drivers_probe` are driven;
  `drivers_autoprobe` is only read), the glue-directory lifecycle, the rejected
  states, the hook queue, and the two properties the attribute rebuild rests on:
  that IDs survive a bind, and that a layer cannot shadow a core attribute. The
  tests assert paths, symlink targets and attribute contents, not just success
  codes.
- **Not implemented: uevent delivery.** Events are built, sequenced and handed to
  the hook, whose kernel-side implementation logs them and stops there.
- **What that means for `udevd`.** A real `udevd` on this kernel would find the
  `/sys` layout it expects and could enumerate devices by walking it, but would
  never receive an event at all. Writing an action to a `uevent` file re-originates
  the event inside the kernel and reaches the same stub, so `udevadm trigger`
  delivers nothing either; no event leaves the kernel until delivery is wired up.

## Limitations

These are behaviors Linux has that this design either cannot express or does not
yet implement. Each is a deliberate stopping point, not an oversight.

**Attributes are flat and textual.** Linux's attribute groups may be *named*, which
makes them subdirectories such as `power/` and `statistics/`, and may hide entries
per device through `is_visible`. Neither is modeled, which is also why `power/` is
absent. Linux's binary attributes, which PCI uses for `config` and `resource<N>`,
have no counterpart either, because `Attr<D>`'s callbacks are text-oriented.

**Only devices get attributes, and only static ones.** `Dir::with_attrs` is
crate-private and only bus and driver registration call it, so the four control
files are the complete set of subsystem-directory files: a `Bus` cannot add a
bus-wide attribute, a `Class` a class-wide one, or a `Driver` anything to its own
directory, where Linux has `bus_attribute`, `class_attribute` and
`driver_attribute`. Every attribute list is `&'static` as well, so there is no
counterpart to `device_create_file` for a single device instance.

**A subsystem cannot add a subdirectory or a symlink of its own to a device
directory.** Linux does this with plain kobjects and `sysfs_create_link`, and the
next two conversions need it: a disk has `holders/`, `slaves/`, `queue/` with
`queue/iosched/` inside it, and a `bdi` link to its backing-device info; a network
interface has `queues/`, one directory per queue.

**No uevent suppression, and only `add` and `remove` are originated.** Linux's
block layer holds back a disk's `add` event across the partition scan
(`dev_set_uevent_suppress`) so that udev sees a complete device. Nothing here can do
that, and nothing in the kernel emits `change`, `bind` or `unbind`; those three
actions exist in the `UeventAction` enum, but only a write to a `uevent` file
produces one.

**Renaming and moving are not supported.** Linux's `device_rename` and
`device_move` work partly because kernfs computes symlink targets at read time, so
links pointing *at* the device follow it, and partly because they patch the rest by
hand: `device_rename` renames the `/sys/class/<c>/<name>` entry, and `device_move`
recreates the `device` link. Even then the bus and driver index entries keep the
old name, which is why the kernel's own comment calls `device_rename` racy. Here
targets are fixed at creation, so neither half is available. The `net` class, whose
interfaces udev renames, will need one or the other.

**`remove` refuses a device with children** rather than orphaning them, which Linux
tolerates. Hot-unplug of a subtree will need a recursive form.

**A bus or a class cannot be unregistered.** The registry keeps every registered
subsystem for the life of the kernel. Drivers and class interfaces can come and go;
their subsystems cannot. Linux allows it because its subsystems can be modules.

**Smaller gaps.** Node ownership (`DEVUID`, `DEVGID`) is not modeled, so every node
is owned by root. The `uevent` file's `store` accepts an action name only; Linux
also accepts a synthetic-event id and `KEY=VALUE` arguments, which reach listeners
as `SYNTH_UUID` and `SYNTH_ARG_KEY=VALUE`. Bus-side interfaces (`subsys_interface`)
are not modeled. A bus cannot supply a root directory for its parentless devices,
as `bus_get_dev_root` does for `cpu`. `driver_override` and deferred probing are
absent. The `uevent` file on bus and driver directories, the `/sys/block` index,
and the `online`, `removable` and `waiting_for_supplier` attributes are missing. A
value written to an attribute is truncated to one page, as kernfs's
`kernfs_fop_write_iter` does for a sysfs attribute.

**An attribute callback can outlive its attribute.** `AttrTable::show` releases the
table lock before running the callback, so a read already in flight can finish
after `remove_attrs` has taken the attribute away. This is discussed below.

## Discussions

### Why not one erased device, as in Linux?

It would work, and it would be less code. What it would not do is prevent the
mistakes: a device with both a bus and a class, a driver registered on a bus it was
not written for, an attribute attached to the wrong kind of device. Each of those
is a runtime check or a silent no-op in C, and each of them is a class of bug that
Rust can remove entirely for the price of one type parameter. R17 asks for that
price to be paid.

### Why not type the whole tree?

Because the tree is heterogeneous and open. A PCI device's children can be a virtio
device, a class device and a glue directory; a `mem` device's parent is a plain
directory. Typing the parent and child edges forces a closed enum of child kinds
that every new driver would have to extend, and it duplicates the registration
sequence per instantiation. Erasing exactly those two edges, and nothing else, is
what makes one `add` serve every subsystem.

### Why is the class device a separate device?

Because Linux's model is not "a device may be in a bus and a class" but "a device
is in one subsystem, and a driver may create another device to be its user-facing
face". The virtio disk is `virtio0` on the virtio bus plus `vda` in the `block`
class, parent and child. Following that gives the right sysfs layout for free, and
it is also the honest description of the objects: they have different lifetimes,
different attributes, and different removal rules.

### Why does the model own `/dev` node creation?

Because the alternative is what Asterinas has today: the node is a side effect of
registering in a number registry, so nothing guarantees that it appears exactly
once, at the right time, with the right mode, or that it disappears on removal. Now
`add` creates it in step 5 and `teardown` deletes it, and the same computed request
feeds `DEVNAME` and `DEVMODE`, so the file and the announcement cannot disagree.

### Why hooks rather than a direct call?

The component layering forbids a component from depending on the kernel crate
(R16), and devtmpfs and the netlink socket live there. Inverting the dependency
keeps the layering intact and has a side benefit: the queue-and-replay behavior
becomes testable, because the tests can install their own hook slot instead of the
global one.

### Why does a device model have two uevent vocabularies?

It should not, and eventually it will not. The kernel crate's netlink code already
defines a `Uevent` with an action enum and a sequence counter, for the messages
user space sends and receives; this crate defines its own because a component
cannot depend on the kernel crate. When delivery is wired up, the netlink side
should build its wire form from this type, leaving one definition. The wire form
itself is not what this type writes: a netlink message is a NUL-terminated
`<action>@<devpath>` header followed by NUL-terminated `KEY=value` strings.

Delivery will also have to decide how much ordering to promise. Linux promises
less than one might assume: `SEQNUM` comes from an atomic counter taken outside
any lock (`lib/kobject_uevent.c`, `atomic64_inc_return(&uevent_seqnum)`), and
`uevent_sock_mutex` serializes only the untagged broadcast loop, while the
network-namespace-tagged path takes no lock at all, so two events can be numbered
in one order and delivered in another. Asterinas can either copy that, or take the
number and send the message under one lock and promise more than Linux does; the
choice should be deliberate rather than incidental.

### Why can an attribute callback outlive its attribute?

Because the alternative is worse. Holding the table's read lock across the callback
would close the window, and it would also mean that a callback which binds a driver
takes the same table's write lock while holding the read lock: a deadlock. It would
also break the rule that no callback in this crate runs under a lock of the model.
The behavior is memory-safe, because the caller holds the device alive through an
`Arc`; what a reader can observe is a value from an attribute that has just been
removed. Linux avoids even that by draining active references in `kernfs_drain`,
which is a mechanism `systree` does not have and would be a larger change than this
design warrants.

### What does this cost at runtime?

Two costs are worth naming, because the design deliberately accepts them.

The attribute table is per device, not shared. Linux points every device of a kind
at one static `attribute_group` (`device_add_groups` walks `class->dev_groups` and
friends without copying); here, registration erases each typed attribute
into a `TyErasedAttr`, which allocates one `Arc`ed closure for a read-only attribute
and two for a read-write one, and inserts it into a `BTreeMap` on the device. A
device with ten attributes, most of them read-only, therefore costs a dozen or so
small allocations plus a map, where Linux shares one static table across every
device of the kind. Linux still allocates a `kernfs_node` per attribute file at
`device_add` time; this design creates its sysfs inodes lazily on lookup instead,
so the trade is eager erasure against eager inodes, not allocation against nothing.

What buys the erasure is the type safety: the closure is where the erased
`&dyn AnyDevice` is turned back into the concrete device the callback was written
for. If it ever matters, the erasure
can be hoisted into a per-`(layer, type)` static built once, since the closures
depend only on the types, not on the device.

Lookups are linear. A bus keeps its devices and drivers in `Vec`s behind a mutex,
so `find_device` is O(n), and every `add` walks every driver of the bus. For a
machine with a handful of virtio devices this is not worth a map; for a bus with
hundreds of devices it would be, and the change is local to `BusHandle`.

### Why refuse to remove a device with children?

Linux allows it: `device_del` does not look at the child list, and the children's
directories vanish with the parent's while their `struct device`s live on. That is
a convention maintained by every driver's `remove`, not a guarantee. Refusing is
strictly safer and costs the caller one rule — unbind, then remove — which is what
a driver's `remove` already does. If hot-unplug of a subtree ever needs it, a
recursive variant can be added; the reverse, weakening a guarantee, would be harder.

### Why is `is_added` true during `Adding`?

Because a driver's `probe` runs inside its device's `add`, and a driver that
creates a class device for the device it just probed would otherwise be rejected
for having an unregistered parent. Treating `Adding` as added supports exactly that
re-entrant case; it does not license another thread to add children to a device
whose registration is still in flight, and the design does not claim it does.

### Why keep the number registries at all?

They answer a different question. The device model answers "what devices exist and
how are they arranged"; the char and block registries answer "which object does
this device number open", which is what a `open("/dev/null")` needs and what Linux
keeps a separate `cdev` map for. Merging them would tie a lookup on the hot path to
a tree walk. What did change is that the registry no longer creates `/dev` nodes.

## Future work

In the order the work is likely to be needed:

1. **Uevent delivery.** Connect `broadcast_uevent` to the existing netlink
   multicast path with the framing above; emit `bind` and `unbind` in `try_bind`
   and `unbind` (Linux's `zap_modalias_env` strips `MODALIAS` from `unbind`, so
   that udev does not reload a module); accept arguments and a synthetic-event id
   on writes to `uevent`.
2. **Real buses.** A PCI bus with a `pci0000:00` bare root, a virtio bus whose
   devices are children of their PCI transports, and the platform bus for
   MMIO-described devices. `BusDevice<Pci>` carries the configuration-space handle
   the drivers already use, so the conversion is mostly moving the enumeration code
   behind `add`.
3. **The `block` and `input` classes.** Convert virtio-blk and the input devices,
   with partitions as a device type that drops the `device` link. Partition
   scanning belongs in the disk driver's probe, where Linux does it
   (`device_add_disk`), not in a class interface, which may not add devices of its
   own class.
4. **Subsystem-owned subdirectories,** which the block class needs for `holders/`,
   `slaves/` and `queue/`, and named attribute groups, which everything needs for
   `power/`.
5. **`driver_override` and deferred probing,** both additions to `BusHandle`: an
   attribute per device, and a retry list.
6. **A class that keeps its glue directory.** The placement is built and tested but
   no class sets `KEEPS_GLUE_DIR` yet; `net` will be the first.
