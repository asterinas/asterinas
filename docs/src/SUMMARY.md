# Summary

[Introduction](introduction.md)

# Asterinas Kernel

* [Getting Started](kernel/README.md)
* [A Zero-Cost, Least-Privilege Approach](kernel/the-approach/README.md)
    * [Framekernel OS Architecture](kernel/the-approach/framekernel.md)
    * [Component-Level Access Control](kernel/the-approach/components.md)
    * [Type-Level Capabilities](kernel/the-approach/capabilities.md)
* [Development Status and Roadmap](kernel/status-and-roadmap.md)
* [Linux Compatibility](kernel/linux/README.md)
    * [File Systems](kernel/linux/file-systems.md)
    * [Networking](kernel/linux/network.md)
    * [Boot Protocols](kernel/linux/boot.md)

# Asterinas Framework

* [An Overview of Framework APIs](framework/README.md)
* [Writing a Kenrel in 100 Lines of Safe Rust](framework/an-100-line-example.md)

# Asterinas OSDK

* [OSDK User Guide](osdk/guide/README.md)
    * [Why OSDK](osdk/guide/why.md)
    * [Creating an OS Project](osdk/guide/create-project.md)
    * [Testing or Running an OS Project](osdk/guide/run-project.md)
    * [Working in a Workspace](osdk/guide/work-in-workspace.md)
* [OSDK User Reference](osdk/reference/README.md)
    * [Commands](osdk/reference/commands/README.md)
        * [cargo osdk new](osdk/reference/commands/new.md)
        * [cargo osdk build](osdk/reference/commands/build.md)
        * [cargo osdk run](osdk/reference/commands/run.md)
        * [cargo osdk test](osdk/reference/commands/test.md)
    * [Manifest](osdk/reference/manifest.md)

# How to Contribute

* [Before You Contribute](to-contribute/README.md)
* [Code Organization](to-contribute/code-organization.md)
* [Style Guidelines](to-contribute/style-guidelines/README.md)
    * [General Guidelines](to-contribute/style-guidelines/general-guidelines.md) 
    * [Rust Guidelines](to-contribute/style-guidelines/rust-guidelines.md) 
    * [Git Guidelines](to-contribute/style-guidelines/git-guidelines.md) 
* [Community](to-contribute/community.md)
* [Code of Conduct](to-contribute/code-of-conduct.md)

# Request for Comments (RFC)

* [RFC Overview](rfcs/README.md)
    * [RFC-0001: RFC Process](rfcs/0001-rfc-process.md)
    * [RFC-0002: Operating System Development Kit (OSDK)](rfcs/0002-osdk.md)
