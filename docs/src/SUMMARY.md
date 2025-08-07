# Summary

[Introduction](README.md)

# Asterinas Kernel

* [Getting Started](kernel/README.md)
* [Advanced Build and Test Instructions](kernel/advanced-instructions.md)
    * [Intel TDX](kernel/intel_tdx.md)
* [The Framekernel Architecture](kernel/the-framekernel-architecture.md)
* [Linux Compatibility](kernel/linux-compatibility/README.md)
    * [Limitations on System Calls](kernel/linux-compatibility/limitations-on-system-calls/README.md)
        * [System Call Matching Language (SCML)](kernel/linux-compatibility/limitations-on-system-calls/system-call-matching-language.md)
        * [Process and thread management](kernel/linux-compatibility/limitations-on-system-calls/process-and-thread-management.md)
        * [Memory management](kernel/linux-compatibility/limitations-on-system-calls/memory-management.md)
        * [File & directory operations](kernel/linux-compatibility/limitations-on-system-calls/file-and-directory-operations.md)
        * [File systems & mount control](kernel/linux-compatibility/limitations-on-system-calls/file-systems-and-mount-control.md)
        * [File descriptor & I/O control](kernel/linux-compatibility/limitations-on-system-calls/file-descriptor-and-io-control.md)
        * [Inter-process communication](kernel/linux-compatibility/limitations-on-system-calls/inter-process-communication.md)
        * [Networking & sockets](kernel/linux-compatibility/limitations-on-system-calls/networking-and-sockets.md)
        * [Signals & timers](kernel/linux-compatibility/limitations-on-system-calls/signals-and-timers.md)
        * [Namespaces, cgroups & security](kernel/linux-compatibility/limitations-on-system-calls/namespaces-cgroups-and-security.md)
        * [System information & misc](kernel/linux-compatibility/limitations-on-system-calls/system-information-and-misc.md)
    * [Limitations on File Systems]()
* [Roadmap](kernel/roadmap.md)

# Asterinas OSTD

* [An Overview of OSTD](ostd/README.md)
* [Example: Writing a Kernel in 100 Lines of Safe Rust](ostd/a-100-line-kernel.md)
* [Example: Writing a Driver in 100 Lines of Safe Rust]()
* [Soundness Analysis]()

# Asterinas OSDK

* [OSDK User Guide](osdk/guide/README.md)
    * [Why OSDK](osdk/guide/why.md)
    * [Creating an OS Project](osdk/guide/create-project.md)
    * [Testing or Running an OS Project](osdk/guide/run-project.md)
    * [Working in a Workspace](osdk/guide/work-in-workspace.md)
    * [Advanced Topics](osdk/guide/advanced_topics.md)
        * [Intel TDX](osdk/guide/intel-tdx.md)
* [OSDK User Reference](osdk/reference/README.md)
    * [Commands](osdk/reference/commands/README.md)
        * [cargo osdk new](osdk/reference/commands/new.md)
        * [cargo osdk build](osdk/reference/commands/build.md)
        * [cargo osdk run](osdk/reference/commands/run.md)
        * [cargo osdk test](osdk/reference/commands/test.md)
        * [cargo osdk debug](osdk/reference/commands/debug.md)
        * [cargo osdk profile](osdk/reference/commands/profile.md)
    * [Manifest](osdk/reference/manifest.md)

# How to Contribute

* [Before You Contribute]()
* [Code Organization]()
* [Style Guidelines]()
    * [General Guidelines]() 
    * [Rust Guidelines](to-contribute/style-guidelines/rust-guidelines.md) 
    * [Git Guidelines]() 
* [Boterinas](to-contribute/boterinas.md)
* [Version Bump](to-contribute/version-bump.md)
* [Community]()
* [Code of Conduct]()

# Request for Comments (RFC)

* [RFC Overview]()
    * [RFC-0001: RFC Process]()
    * [RFC-0002: Operating System Development Kit (OSDK)]()
