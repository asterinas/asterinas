
# CapComp: Towards a Zero-Cost Capability-Based Component System for Rust

## Overview

**CapComp** is a zero-cost capability-based component system for Rust. With CapComp, Rust developers can now build _component-based systems_ that follow the _principle of least privilege_, which is effective in improving system security and stability. CapComp can also make formal verification of Rust systems more feasible since it reduces the Trusted Computing Base (TCB) for a specific set of properties. We believe CapComp is useful for building complex systems of high security or stability requirements, e.g., operating systems.

CapComp features a novel _zero-cost access control_ mechanism. Unlike the traditional approach to capability-based access control (e.g., seL4), CapComp enforces access control at compile time: no runtime check for access rights, no hardware isolation overheads, or extra runtime overheads of Inter-Procedural Calls (IPC). To achieve this, CapComp has realized the full potential of the Rust language by making a joint and clever use of new type primitives, type-level programming, procedural macros, and program analysis techniques.

CapComp is designed to be _pragmatic_. To enjoy the benefits of CapComp, developers need to design new systems with CapComp in mind or refactor existing systems accordingly. But these efforts are minimized thanks to the intuitive and user-friendly APIs provided by CapComp. Furthermore, CapComp supports incremental adoption: it is totally ok to protect only selected parts of a system with CapComp. Lastly, CapComp does not require changes to the Rust language or compiler.

## Motivating examples

To illustrate the motivation of CapComp, let's first examine two examples. While both examples originate in the context of OS, the motivations behind them make sense in other contexts.

### Example 1: Capabilities in a microkernel-based OS 

A capability is a communicable, unforgeable token of authority. Conceptually, a capability is a reference to an object along with an associated set of access rights. Capabilities are chosen to be the core primitive that underpins the security of many systems, most notably, several microkernels like seL4, zircon, and Capsicum.

In Rust, the most straightforward way to represent a capability is like this (as is done in zCore, a Rust rewrite of zircon). In other languages like C and C++, the representation is similar.

```rust
/// A handle = a capability.
pub struct Handle<O> {
    /// The kernel object referred to by the handle.
    pub object: Arc<O>,
    /// The access rights associated with the handle.
    pub rights: Rights,
}
```

The problem with this approach is three-fold. First, a capability (i.e., `Handle`) consumes more memory than a raw pointer. Second, enforcing access control according to the associated rights requires runtime checking. Third, due to the overhead of runtime checking, users of `Handle` have the incentive to skip the runtime checking whenever possible and use the object directly. Thus, we cannot rule out the odds that some buggy code fails to enforce the access control, leading to security loopholes.

So, here is our question: **is it possible to implement capabilities in Rust without runtime overheads and (potential) security loopholes?**

### Example 2: Key retention in a monolithic OS kernel

Let's consider a key retention subsystem named `keyring` in a monolithic kernel (such a subsystem exists in the Linux kernel). The subsystem serves as a secure and centralized place to manage secrets used in the OS kernel. The users of the `keyring` subsystem include both user programs and kernel services.

Here is an (overly-)simplified version of `keyring` in Rust.

```rust
/// A key is some secret data.
pub struct Key(Vec<u8>);

impl Key {
    pub fn new(bytes: &[u8]) -> Self {
        Self(Vec::from(bytes))
    }

    pub fn get(&self) -> &[u8] {
        self.0.as_slice()
    }

    pub fn set(&mut self, bytes: &[u8]) -> &[u8] {
        self.0.clear();
        self.0.extend_from_slice(bytes);
    }
}

/// Store a key with a name.
pub fn insert_key(name: &str, key: Key) {
    KEYS.lock().unwrap().insert(name.to_string(), Arc::new(key))
}

/// Get a key by a name.
pub fn get_key(name: &str) -> Option<Arc<Key>> {
    KEYS.lock()
        .unwrap()
        .get(&name.to_string())
        .map(|key| key.clone())
}

lazy_static! {
    static ref KEYS: Mutex<HashMap<String, Arc<Key>> = {
        Mutex::new(HashMap::new())
    };
}
```

While any components (or modules/subsystems/drivers) in a monolithic kernel are part of the TCB and thus trusted, it is still highly desirable to constraint the access of keys to only the components that have some legitimate reasons to use them. This minimizes the odds of misusing or leaking the highly sensitive keys maintained by `keyring`.

Our question is that **how to enforce inter-component access control (as demanded here by `keyring`) in a complex system like a monolithic kernel and without runtime overheads?**

## Core concepts

Before demonstrating how we approach the above problems in CapComp, we need to first introduce some core concepts in CapComp.

In CapComp, **components** are Rust crates that are governed by our compile-time capability-based access control mechanism. Crates that maintain mutable states or have global impacts on a system are good candidates to be protected as components. 
Unlike a regular crate, a component can decide how many functionalities to expose on a per-component or per-object basis; that is, only granting to its users the bare minimal privileges necessary to do their jobs.

For our purpose of access control, we define the **entry points** of a component as the following types of _public Rust language items_ exposed by a component.
* Structs, enumerations, and unions;
* Functions and associated functions;
* Static items;
* Constant items.

We consider these items as entry points because a user who gets access to such items can directly _execute_ the code within the component. And also for this reason, other types of Rust language items (e.g., modules, type aliases, use declarations, traits, etc.) are not considered as entry points: they either do not involve direct code execution (like modules) or only involves _indirect_ code execution (like type aliases, use declarations, and traits) via other entry points.

But not all entry points are created equal. Some are actually safe to use arbitrarily without jeopardizing other parts of the system, while some are considered **privileged** in the sense that _uncontrolled_ access to such entry points may have undesirable impacts on the system. The opposite of privileged entry points is the **unprivileged** ones. CapComp has a set of built-in rules to identify some common patterns of naive or harmless code that can be safely considered as unprivileged. In the meantime, developers can decide which entry points they believe are unprivileged and point them out for CapComp. The rest of the entry points are thus the privileged ones. Put it in another way, all entry points are considered privileged by default; unprivileged ones are the exceptions. Privileged entry points are the targets of the access control mechanism of CapComp.

With the concepts explained, we can now articulate the three main properties of CapComp.
* **Object-level access control.** All public methods of an object behind a capability are access controlled.
* **Component-level access control.** All privileged entry points of a component are accessed controlled.
* **Mandatory access control.** All users of components cannot penetrate the access control without using the `unsafe` keyword.

These properties are mostly enforced using compile-time techniques, thus incurring zero runtime overheads.

## Usage

To give you a concrete idea of CapComp, we revisit the two motivating examples and show how CapComp can meet their demands.

### Example 1: Capabilities in a microkernel-based OS 

One fundamental primitive provided by CapComp is capabilities. The type that represents a capability is `Cap<O, R>`, where `O` is the type of the capability's associated object and `R` is the type of the capability's associated rights. 

```rust
// File: cap_comp/cap.rs

use core::marker::PhantomData;

/// A capability.
#[repr(transparent)]
pub struct Cap<O: ?Sized, R> {
    rights: PhantomData<R>,
    object: O,
}

impl<O, R> Cap<O, R> {
    pub fn new(object: O) -> Self {
        Self {
            rights: PhantomData,
            object,
        }
    }
}
```

Notice how the Rust representation of a capability in CapComp differs from the traditional approach. The key difference is that CapComp represents rights in _types_, not values. This enables CapComp to enforce access control at compile time.

So how do users use CapComp's capabilities? Imagine you are developing a kernel object named `Endpoint`, which can be used to send and receive bytes between threads or processes. 

```rust
// File: user_project/endpoint.rs

pub struct Endpoint;

impl Endpoint {
    pub fn recv(&self) -> u8 {
        todo!("impl recv")
    }

    pub fn send(&self, _byte: u8) {
        todo!("impl send")
    }
}
```

You want to ensure that the sender side of an `Endpoint` (could be a process or thread) can only use it to send bytes, while the receiver side can only receive bytes. We can do so easily with CapComp.

First, add `cap_comp` crate as a dependency in your project's `Cargo.toml`.

```rust
[dependencies]
cap_comp = { version = "0.1", features = ["read_right", "write_right"] }
```

The `features` field specifies which kinds of access rights that your project finds interesting. There are a rich set of general-purpose access rights built into CapComp and you can select those that make sense to your project. (We may find a way to support customized access rights in the future.)

Then, we can refactor the code of `Endpoint` to leverage CapComp to protect `Endpoint` with capabilities.

```rust
// File: user_project/endpoint.rs

use cap_comp::{
    rights::{Read, Write},
    impl_cap, require,
	Cap, Rights,
};

pub struct Endpoint;

// Instruct CapComp to implement the right set of methods for
// `Cap<Endpoint, R>`, depending upon rights specified by `R`.
#[impl_cap]
impl Endpoint {
    // Implement the recv method for `Cap<Endpoint, R: Read>`.
    #[require(rights = [Read])]
    pub fn recv(&self) -> u8 {
        todo!("impl recv")
    }

    // Implement the send method for `Cap<Endpoint, R: Write>`.
    #[require(rights = [Write])]
    pub fn send(&self, _byte: u8) {
        todo!("impl send")
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn recv() {
        // Create an endpoint capability with the write right.
        let endpoint_cap: Cap<Endpoint, Rights![Write]> = Cap::new(Endpoint);
        endpoint_cap.send(0);
        //endpoint_cap.recv(); // compiler error!
    }

    #[test]
    fn send() {
        // Create an endpoint capability with the read right.
        let endpoint_cap: Cap<Endpoint, Rights![Read]> = Cap::new(Endpoint);
        endpoint_cap.recv();
        //endpoint_cap.send(0); // compiler error!
    }
}
```

The above code shows the most basic usage of capabilities and rights in CapComp and CapComp's ability to enforce object-level access control at compile time.

### Example 2: Key retention in a monolithic OS kernel

#### What are tokens?

Just like the methods of an object can be access-controlled wtih capabilities, the entry points of a component can be access-controlled with tokens. In CapComp, a **token** is owned by a component or a module inside a component; such a component or module is called **token owner**. A token represents the access rights of the token owner to other components. So by presenting its token, a token owner can prove to a recipient component which access rights it possesses; and based on token, the recipient component can decide whether to allow a specific operation accordingly.

Anywhere inside a component, a user can get access to the token that belongs to the current textual scope via `Token!()`, a special macro provided by CapComp. The macro returns a _zero-sized_ Rust object of type `T: Token`, where `Token` is a marker trait for all token types. Type `T` is encoded with the access rights info via type-programming techniques. Thus, transferring and checking tokens can be done entirely at the compile time, without incurring any runtime cost.

#### Refactor `keyring` with tokens

To make our `keyring` example more realistic, let's consider a system that consists of a `keyring` component and three other components.
* The `keyring` component is responsible for key management.
* The `boot` component parses arguments and configurations (including encryption keys) from users and starts up the entire system.
* The `encrypted_fs` component implements encrypted file I/O, which uses encryption keys.
* The `proc_fs` component implements a Linux-like procfs that facilitate inspecting the status of the system (e.g., all available keys).

To leverage the component-level access control mechanism of CapComp, one needs to provide a configuration file for CapComp, which specifies all components in the target system and their access rights.

```toml
# File: CapComp.toml

# There are four components in total
[components]
keyring = { path = "keyring/" }
boot = { path = "boot/" }
encrypted_fs = { path = "encrypted_fs/" }
procfs = { path = "procfs/" }

# A centralized place to claim access to the keyring component
[access_to.keyring]
# The boot component has the write right. 
boot = { rights = ["Write"] }

# The init module inside the encrypted_fs component needs 
# the read right
'encrypted_fs.init' = { rights = ["Read", "Inspect"] }

# The proc_fs component has the inspect right
proc_fs = { rights = ["Inspect"] }
```

Now we refactor the `keyring` component with CapComp.

```rust
// File: keyring/lib.rs

use cap_comp::{
	load_config, impl_cap, require, unprivileged
	Token,
}

load_config!("../CapComp.toml");

/// A key is some secret data.
// Inform CapComp that this type is unprivileged, which means
// it does not have to be protected by capabilities.
#[unprivileged]
pub struct Key {
    name: String,
    payload: Vec<u8>,
}

#[impl_cap]
impl Key {
    pub fn new(name: &str, bytes: &[u8]) -> Self {
        Self {
            name: name.to_string(),
            payload: Vec::from(bytes),
        }
    }

    #[require(rights = [Inspect])]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[require(rights = [Read])]
    pub fn payload(&self) -> &Vec<u8> {
        &self.payload
    }

    #[require(rights = [Write])]
    pub fn payload_mut(&mut self) -> &mut Vec<u8> {
        &mut self.payload
    }
}

// A component can access this operation only if it has a token
// with the write right.
#[require(bounds = { Tk: Write })]
pub fn insert_key<Tk>(key: Key, token: &Tk) {
    todo!("omitted...")
}

// A component can access this operation only if it has a token
// with the write right.
#[require(bounds = { Tk: Read })]
pub fn find_key<Tk>(name: &str, token: &Tk) -> Option<Arc<Cap<Key, Rights![Read]>>> {
    todo!("omitted...")
}

// A component can access this operation only if it has a token
// with the inspect right.
#[require(bounds = { Tk: Inspect })]
pub fn list_all_keys<Tk>(name: &str, token: &Tk) -> Option<Arc<Cap<Key, Rights![Inspect]>>> {
    todo!("omitted...")
}
```

The `boot` component can insert keys into `keyring` since it has the write right.

```rust
// File: boot/lib.rs

use capcomp::{load_config, Token};
use keyring::{insert_key, Key};

load_config!("../CapComp.toml");

fn parse_cli_arguments(args: &[&str]) {
    let key = parse_encryption_key(args);
    insert_key("encryption_key", Token!());
}

fn parse_encryption_key(args: &[&str]) {
    todo!("omitted...")
}
```

The `encrypted_fs` component can fetch and use its encryption key from `keyring` since it has the read right.

```rust
// File: mount_fs/lib.rs
capcomp::load_config!("../CapComp.toml");
```

```rust
// File: mount_fs/init.rs
use capcomp::Token;
use keyring::find_key;

pub fn init_fs() {
    let key = find_key("encryption_key", Token!());
    // use the key to init fs...
}
```

For debug purposes, the `proc_fs` component is allowed to list the metadata of keys since it has the inspect right, but not read or update keys.

```rust
// File: proc_fs/lib.rs

use capcomp::{load_config, Token};
use keyring::list_all_keys;

load_config!("../CapComp.toml");

pub fn list_all_key_names() -> Vec<String> {
    let keys = list_all_keys(Token!());
    key.iter().map(|key| key.name().to_string()).collect()
}
```

As you can see, the CapComp-powered system has greatly narrowed the scope of code that has access to keys. In fact, the scope is clearly defined in `CapComp.toml`. This greatly reduces the odds of misusing or leaking keys and facilitates security auditing for the codebase.

## Prototype

We have implemented a prototype of CapComp that can support the two examples above. More specifically, the prototype demonstrates that the following key design points of CapComp are feasible.

* Using Rust's type-level programming to encode access rights with types and enforce access control at compile time (`Cap<O, R>`, `Rights!`, `Token!`).
* Using Rust's procedure macros to minimize the boilerplate code required (`#[impl_cap]`).
* Using program analysis technique (a lint pass) to check if all privileged entry points of a component are access-controlled (`#[unprivileged]`).

## Discussions

* Figure out the killer apps of CapComp
* How to demonstrate the security benefits brought by CapComp?
* Is it too complex for developers to write Rust code with CapComp?
* What is it like to build a component-based OS with CapComp?
* Can we formally verify the security guarantees of CapComp?
