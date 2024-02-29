# Accelerate OS development with Asterinas OSDK

[![Crates.io](https://img.shields.io/crates/v/cargo-osdk.svg)](https://crates.io/crates/cargo-osdk)
[![OSDK Test](https://github.com/asterinas/asterinas/actions/workflows/osdk_test.yml/badge.svg?event=push)](https://github.com/asterinas/asterinas/actions/workflows/osdk_test.yml)

### What is it?

OSDK (short for Operating System Development Kit) is designed to simplify the development of Rust operating systems. It aims to streamline the process by leveraging the framekernel architecture, originally proposed by [Asterinas](https://github.com/asterinas/asterinas).

`cargo-osdk` is a command-line tool that facilitates project management for those developed on the framekernel architecture. Much like Cargo for Rust projects, `cargo-osdk` enables building, running, and testing projects conveniently.

### Install the tool

#### Requirements

Currenly, `cargo-osdk` only supports x86_64 ubuntu system. 

To run a kernel with QEMU, `cargo-osdk` requires the following tools to be installed: 
- Rust >= 1.75.0
- cargo-binutils
- gcc
- qemu-system-x86_64
- grub-mkrescue
- ovmf 
- xorriso

About how to install Rust, you can refer to the [official site](https://www.rust-lang.org/tools/install).

After installing Rust, you can install Cargo tools by
```bash
cargo install cargo-binutils
```

Other tools can be installed by
```bash
apt install build-essential grub2-common qemu-system-x86 ovmf xorriso
```

#### Install 

Then, `cargo-osdk` can be installed by
```bash
cargo install cargo-osdk
``` 

#### Upgrade

If `cargo-osdk` is already installed, the tool can be upgraded by
```bash
cargo install --force cargo-osdk
```

### Get start

Here we provide a simple demo to demonstrate how to create and run a simple kernel with `cargo-osdk`.

With `cargo-osdk`, a kernel project can be created by one command
```bash
cargo osdk new --kernel my-first-os
```

Then, you can run the kernel with
```bash
cd my-first-os && cargo osdk run
```

You will see `Hello world from guest kernel!` from your console. 

### Basic usage

The basic usage of `cargo-osdk` is
```bash
cargo osdk <COMMAND>
```
Currently we support following commands:
- **new**: Create a new kernel package or library package
- **build**: Compile the project and its dependencies
- **run**: Run the kernel with a VMM
- **test**: Execute kernel mode unit test by starting a VMM
- **check**: Analyze the current package and report errors
- **clippy**: Check the current package and catch common mistakes

The following command can be used to discover the available options for each command.
```bash
cargo osdk help <COMMAND>
```

### The configuration file

`cargo-osdk` utilizes a configuration file to define its precise behavior. Typically, the configuration file is named `OSDK.toml` and is placed in the root directory of the workspace (the same directory as the workspace's `Cargo.toml`). If there is only one crate and no workspace, the file is placed in the crate's root directory. Below, you will find a comprehensive version of the available configuration options.

```toml
kcmd_args = ["init=/bin/busybox", "path=/usr/local/bin"] # <1>
init_args = ["sh", "-l"] # <2>
initramfs="./build/initramfs.cpio.gz" # <3>

[boot]
loader = "grub" # <4>
protocol = "multiboot2" # <5>
grub-mkrescue = "/usr/bin/grub-mkrescue" # <6>
ovmf = "/usr/bin/ovmf" # <7>

[qemu]
path = "/usr/bin/qemu-system-x86_64" # <8>
machine = "q35" # <9>
args = [ # <10>
    "-enable-kvm",
    "-m 2G", 
    "-device virtio-keyboard-pci,disable-legacy=on,disable-modern=off"
] 

[qemu.'cfg(feature="iommu")'] # <11>
path = "/usr/local/sbin/qemu-kvm" # <8>
machine = "q35" # <9>
args = [ # <10>
    "-enable-kvm",
    "-m 2G", 
    "-device virtio-keyboard-pci,disable-legacy=on,disable-modern=off,iommu_platform=on,ats=on",
    "-device intel-iommu,intremap=on,device-iotlb=on"
] 
```

1. The arguments provided will be passed to the guest kernel.   
Optional. The default value is empty.   
Each argument should be in one of the following two forms: `KEY=VALUE` or `KEY` if no value is required. Each `KEY` can appear at most once.  
2. The arguments provided will be passed to the init process, usually, the init shell.   
Optional. The default value is empty.
3. The path to the built initramfs.  
Optional. The default value is empty.
4. The bootloader used to boot the kernel.  
Optional. The default value is `grub`.   
The allowed values are `grub` and `qemu` (`qemu` indicates that QEMU directly boots the kernel).
5. The boot protocol used to boot the kernel.    
Optional. The default value is `multiboot2`.    
The allowed values are `linux-efi-handover64`, `linux-legacy32`, `multiboot`, and `multiboot2`.
6. The path of `grub-mkrescue`, which is used to create a GRUB CD_ROM.   
Optional. The default value is system path, determined using `which grub-mkrescue`.   
This argument only takes effect when the bootloader is `grub`.   
7. The path of OVMF. OVMF enables UEFI support for QEMU.   
Optional. The default value is empty.   
This argument only takes effect when the boot protocol is `linux-efi-handover64`.
8. The path of QEMU.  
Optional. The default value is system path, determined using `which qemu-system-x86_64`.  
9. The machine type of QEMU.  
Optional. Default is `q35`.  
The allowed values are `q35` and `microvm`.  
10. Additional arguments passed to QEMU.   
Optional. The default value is empty.   
Each argument should be in the form `KEY VALUE` (separated by space), or `KEY` if no value is required. Some keys can appear multiple times (e.g., `-device`, `-netdev`), while other keys can appear at most once. Certain keys, such as `-cpu` and `-machine`, are not allowed to be set here as they may conflict with the internal settings of `cargo-osdk`.  
11. Conditional QEMU settings.   
Optional. The default value is empty.   
Conditional QEMU settings allow for a condition to be specified after `qemu`. Currently, `cargo-osdk` only supports the condition `cfg(feature="FEATURE")`, which activates the QEMU settings only if the `FEATURE` is set. The `FEATURE` must be defined in the project's `Cargo.toml`. At most one conditional setting can be activated at a time. If multiple conditional settings can be activated simultaneously, `cargo-osdk` will report an error. In the future, `cargo-osdk` will support all possible conditions that [Rust conditional compilation](https://doc.rust-lang.org/reference/conditional-compilation.html) supports.