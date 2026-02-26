# Asterinas NixOS

Asterinas NixOS is the first distribution for Asterinas.
We choose [NixOS](https://nixos.org/) as the base OS
for its unparalleled customizability and rich package ecosystem.
Asterinas NixOS is intended to look and feel like vanilla NixOS,
except that it replaces Linux with Asterinas as its kernel.
For more rationale about choosing NixOS,
see [the RFC](https://asterinas.github.io/book/rfcs/0002-asterinas-nixos.html).

**Disclaimer: Asterinas is an independent, community-led project.
Asterinas NixOS is _not_ an official NixOS project and has _no_ affiliation with the NixOS Foundation. _No_ sponsorship or endorsement is implied.**

Asterinas NixOS is not ready for production use.
We provide Asterinas NixOS to make the Asterinas kernel more accessible,
allowing early adopters and enthusiasts to try it out and provide feedback.
In addition, Asterinas developers use this distro
as a key development vehicle
to facilitate enabling and testing more real-world applications
on the Asterinas kernel.

## Getting Started

### End Users

The following instructions describe how to install Asterinas NixOS in a VM
using the Asterinas NixOS installer ISO.

1. Launch an x86-64 Ubuntu container:

    ```bash
    docker run -it --privileged --network=host ubuntu:latest bash
    ```

2. Inside the container, install QEMU:

    ```bash
    apt update
    apt install -y qemu-system-x86 qemu-utils
    ```

3. Download the latest Asterinas NixOS installer ISO from [GitHub Releases](https://github.com/asterinas/asterinas/releases).

4. Create a file that will be used as the target disk to install Asterinas NixOS:

    ```bash
    # The capacity of the disk is 10GB; adjust it as you see fit
    dd if=/dev/zero of=aster_nixos_disk.img bs=1G count=10
    ```

5. Start an x86-64 VM with two drives:
one is the installer CD-ROM and the other is the target disk:

    ```bash
    export INSTALLER_ISO=/path/to/your/downloaded/installer.iso
    qemu-system-x86_64 \
    -cpu host -m 8G -enable-kvm \
    -drive file="$INSTALLER_ISO",media=cdrom -boot d \
    -drive if=virtio,format=raw,file=aster_nixos_disk.img \
    -chardev stdio,id=mux,mux=on,logfile=qemu.log \
    -device virtio-serial-pci -device virtconsole,chardev=mux \
    -serial chardev:mux -monitor chardev:mux \
    -nographic
    ```

    After the VM boots, you now have access to the installation environment.

6. Edit the `configuration.nix` file in the home directory
to customize the NixOS system to be installed:

    ```bash
    vim configuration.nix
    ```
    
    The complete syntax and guidance for the `configuration.nix` file
    can be found in [the NixOS manual](https://nixos.org/manual/nixos/stable/#ch-configuration).
    If you are not familiar with NixOS,
    you can simply skip this step.
    
    Not all combinations of settings in `configuration.nix` are supported by Asterinas NixOS yet.
    The ones that have been tested are documented in the subsequent chapters.

7. Start installation:

    ```bash
    install_aster_nixos.sh --config configuration.nix --disk /dev/vda --force-format-disk
    ```
    
    The installation process involves downloading packages
    and may take around 30 minutes to complete,
    depending on your network speed.

8. After the installation is complete, you can shut down the VM:

    ```bash
    poweroff
    ```
    
    Now Asterinas NixOS is installed in `aster_nixos_disk.img`.

9. Start a VM to boot the newly installed Asterinas NixOS:

    ```bash
    qemu-system-x86_64 \
    -cpu host -m 8G -enable-kvm \
    -bios /usr/share/qemu/OVMF.fd \
    -drive if=none,format=raw,id=x0,file=aster_nixos_disk.img \
    -device virtio-blk-pci,drive=x0,disable-legacy=on,disable-modern=off \
    -chardev stdio,id=mux,mux=on,logfile=qemu.log \
    -device virtio-serial-pci -device virtconsole,chardev=mux \
    -serial chardev:mux -monitor chardev:mux \
    -device virtio-net-pci,netdev=net0,disable-legacy=on,disable-modern=off \
    -netdev user,id=net0 \
    -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
    -nographic -display vnc=127.0.0.1:21
    ```
    
    If a desktop environment is enabled in the `configuration.nix` file,
    you can view the graphical interface using a VNC client.

### Kernel Developers

1. Follow Steps 1 and 2 in the ["Getting Started" section of the Asterinas Kernel](../kernel/index.html#getting-started)
   to set up the development environment.

2. Inside the Docker container,
generate a disk image with Asterinas NixOS installed using this command:

    ```bash
    make nixos
    ```

    or this command:
    
    ```bash
    make iso && make run_iso
    ```

    The difference between the two methods is that
    the first installs NixOS to a disk image entirely inside the container,
    whereas the second emulates the manual ISO installation steps
    (see the [previous section](#end-users))
    by running a VM.
    Using either method results in a disk image with an Asterinas NixOS installation.
    
3. Start a VM to run the installed Asterinas NixOS:

    ```bash
    make run_nixos
    ```
