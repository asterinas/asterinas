# Summary

[Introduction](README.md)

# Asterinas NixOS

* [Getting Started](distro/README.md)
* [Popular Applications](distro/popular-applications/README.md)
    * [Package Management](distro/popular-applications/package-management.md)
    * [Desktop Environment](distro/popular-applications/desktop-environment.md)
    * [Containerization](distro/popular-applications/containerization.md)

# Asterinas Kernel

* [Getting Started](kernel/README.md)
* [Advanced Build and Test Instructions](kernel/advanced-instructions.md)
    * [Intel TDX](kernel/intel-tdx.md)
* [The Framekernel Architecture](kernel/the-framekernel-architecture.md)
* [Linux Compatibility](kernel/linux-compatibility/README.md)
    * [Syscall Flag Coverage](kernel/linux-compatibility/syscall-flag-coverage/README.md)
        * [System Call Matching Language (SCML)](kernel/linux-compatibility/syscall-flag-coverage/system-call-matching-language.md)
        * [Process and thread management](kernel/linux-compatibility/syscall-flag-coverage/process-and-thread-management/README.md)
        * [Memory management](kernel/linux-compatibility/syscall-flag-coverage/memory-management/README.md)
        * [File & directory operations](kernel/linux-compatibility/syscall-flag-coverage/file-and-directory-operations/README.md)
        * [File systems & mount control](kernel/linux-compatibility/syscall-flag-coverage/file-systems-and-mount-control/README.md)
        * [File descriptor & I/O control](kernel/linux-compatibility/syscall-flag-coverage/file-descriptor-and-io-control/README.md)
        * [Inter-process communication](kernel/linux-compatibility/syscall-flag-coverage/inter-process-communication/README.md)
        * [Networking & sockets](kernel/linux-compatibility/syscall-flag-coverage/networking-and-sockets/README.md)
        * [Signals & timers](kernel/linux-compatibility/syscall-flag-coverage/signals-and-timers/README.md)
        * [Namespaces, cgroups & security](kernel/linux-compatibility/syscall-flag-coverage/namespaces-cgroups-and-security/README.md)
        * [System information & misc](kernel/linux-compatibility/syscall-flag-coverage/system-information-and-misc/README.md)
    * [File System Coverage]()
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
    * [Advanced Topics](osdk/guide/advanced-topics.md)
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
    * [Environment Variables](osdk/reference/environment-variables.md)

# How to Contribute

* [Before You Contribute](to-contribute/README.md)
* [Code Organization]()
* [Style Guidelines]()
    * [General Guidelines]()
    * [Rust Guidelines](to-contribute/style-guidelines/rust-guidelines.md)
    * [Git Guidelines]()
    * [Assembly Guidelines](to-contribute/style-guidelines/asm-guidelines.md)
* [Boterinas](to-contribute/boterinas.md)
* [Version Bump](to-contribute/version-bump.md)
* [Community]()
* [Code of Conduct]()

# Request for Comments (RFCs)

* [RFC Overview](rfcs/README.md)
  * [RFC-0001: RFC Process](rfcs/0001-rfc-process.md)
  * [RFC-0002: Asterinas NixOS](rfcs/0002-asterinas-nixos.md)
* [RFC Template](rfcs/rfc-template.md)
