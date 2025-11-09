# RFC-0002: Asterinas NixOS

* Status: Approved
* Pull request: https://github.com/asterinas/asterinas/pull/2584
* Date submitted: 2025-11-14
* Date approved: 2025-12-01

## Summary

This RFC formally proposes the establishment of an Asterinas distribution as a new, top-level sub-project of Asterinas. We intend for this distribution to leverage [NixOS](https://nixos.org/) due to its unparalleled customizability and rich package ecosystem. Accordingly, we propose naming this new sub-project **Asterinas NixOS (AsterNixOS)**.

## Motivations

### What is a "distro"?

In the context of operating systems (OSes), a "distribution" (or "distro") typically refers to a complete OS built around a kernel, complemented by userspace tools, libraries, and applications managed via a package management system. A Linux distro, for example, combines the Linux kernel with a rich userspace. Similarly, an Asterinas distro will pair the Asterinas kernel with a comprehensive userspace environment. For the purpose of this discussion, "distro" will refer broadly to either a Linux or Asterinas-based distro.

### Short-term strategic advantages

Achieving a Minimum Viable Product (MVP) milestone is crucial for Asterinas's maturation. Reaching MVP means that Asterinas is ready for evaluation by early adopters, who expect a seamless experience comparable to mainstream OSes. This includes easy application installation and out-of-the-box functionality. Simply providing a kernel is insufficient; we must deliver a user-friendly experience, which necessitates a full-fledged distro complete with an intuitive package manager.

Furthermore, direct control over an Asterinas distro offers a pragmatic approach to addressing Linux compatibility challenges. While Asterinas is committed to a high degree of Linux compatibility, achieving perfect parity in the near term is impractical. A dedicated distro allows us to configure or patch specific packages to circumvent reliance on advanced or bleeding-edge Linux features that do not have high priority in Asterinas. This significantly reduces pressure on Asterinas kernel developers to implement complex features prematurely, enabling a more focused and stable development roadmap.

Beyond external users, Asterinas developers themselves will greatly benefit from a dedicated distro. Testing complex applications with intricate dependencies is a significant hurdle. While "Hello World" or even medium-sized projects like Nginx or Redis can be manually built and integrated into a disk image, this approach does not scale for complex applications such as Spark, PyTorch, or Chromium. Identifying, understanding, and building every dependency for such projects would divert critical kernel development resources. A distro, by its very nature, abstracts this complexity through its package manager, streamlining the testing and development workflow for all Asterinas developers.

### Long-term ecosystem vision

Looking ahead, we envision a thriving ecosystem of Asterinas distros. This initial, kernel-developer-maintained distro will serve as a vital reference implementation, fostering the creation of diverse Asterinas-based distros. While the Linux world boasts numerous distros, Asterinas-specific distros will be essential for two primary reasons:

Firstly, Asterinas prioritizes Linux compatibility at the external ABI level, not the internal kernel module interface. This means Asterinas cannot load Linux kernel modules (`.ko` files). An Asterinas distro will ensure that all system programs exclusively load Asterinas kernel modules.

Secondly, Asterinas will eventually provide features and value propositions that Linux simply does not have—for example, new system calls, file systems, and device drivers. These benefits only become real when applications can detect and leverage them.

An Asterinas distro is the natural vehicle for this. It can ship new packages written specifically for Asterinas and carry patches to existing packages to make them "Asterinas-aware", ensuring that userspace can meaningfully take advantage of the kernel’s unique capabilities.

In essence, establishing Asterinas distros is paramount for Asterinas's long-term success and differentiation. This RFC lays the groundwork for that future by proposing our foundational distro.

## Design

### Why NixOS as the base distro?

Building the first Asterinas distro from scratch would be an immense undertaking, diverting focus from core kernel development. Therefore, basing it on an existing Linux distro is the most rational path forward. This leads to the crucial question: which Linux distro should serve as our foundation?

The landscape of Linux distros is vast, featuring prominent names such as [Arch](https://archlinux.org/), [CentOS](https://www.centos.org/), [Debian](https://www.debian.org/), and [Gentoo](https://www.gentoo.org/). While theoretically, any of these could serve, we have identified [NixOS](https://nixos.org/) as a particularly attractive and uniquely suited option.

In most distros, package recipes are centered around shell scripts (e.g., [`PKGBUILD`](https://man.archlinux.org/man/PKGBUILD.5#EXAMPLE) in Arch Linux or the [`rules`](https://www.debian.org/doc/manuals/maint-guide/dreq.en.html#rules) file in a Debian package). NixOS, however, utilizes a purpose-built, purely functional language called [Nix](https://nixos-and-flakes.thiscute.world/the-nix-language/), which offers unparalleled expressiveness and flexibility for defining package recipes.

A core requirement for our base distro is the ability to easily tweak existing package recipes to work around Asterinas's limitations or customize the new distro's look or behavior. The Nix language excels in this regard.

For instance, consider overriding specific attributes of an existing Nix package, such as `xfdesktop`, with minimal code:

```nix
{ pkgs }:
{
  xfdesktop = pkgs.xfce.xfdesktop.overrideAttrs (oldAttrs: {
    version = "4.16.0";
    patches = (oldAttrs.patches or []) ++ [
      ./patches/xfdesktop4/0001-Fix-not-using-consistent-monitor-identifiers.patch
    ];
  });
}
```

This Nix file creates a customized `xfdesktop` package without requiring intrusive modifications to its original recipe. This capability, known as [the override design pattern](https://nixos.org/guides/nix-pills/14-override-design-pattern.html) in Nix, is uniquely feasible due to Nix's first-class functions and lazy evaluation.

Another powerful example of NixOS's flexibility is how easily we can create an Asterinas distro ISO installer by customizing an existing [NixOS installer ISO](https://nixos.org/download/#nixos-iso). Consider the following `iso-image.nix` file:

```nix
{ lib, pkgs, ... }:
let
  asterinas_kernel = builtins.path { path = "asterinas_bin"; };
  auto_install_script = pkgs.replaceVarsWith {
    src = "./auto_install.sh";
    isExecutable = true;
    replacements = {
      shell = "${pkgs.bash}/bin/sh";
      inherit asterinas_kernel;
    };
  };
  configuration = {
    imports = [
      "${pkgs.path}/nixos/modules/installer/cd-dvd/installation-cd-minimal.nix"
      "${pkgs.path}/nixos/modules/installer/cd-dvd/channel.nix"
    ];

    services.getty.autologinUser = lib.mkForce "root";
    environment.loginShellInit = "${auto_install_script}";
  };
in (pkgs.nixos configuration).config.system.build.isoImage
```

This file defines a new installer ISO that differs from the original in two key ways. First, it ships with the Asterinas kernel (`asterinas_kernel`). Second, instead of dropping the user into an interactive shell, the ISO's init process immediately runs an automatic installation script (`auto_install.sh`).

A notable NixOS feature relevant to our design is `/etc/nixos/configuration.nix`, the [primary configuration file](https://nixos.org/manual/nixos/stable/#sec-configuration-file) that determines _all_ system-wide states of a NixOS installation, including the kernel, installed software, and configuration. A minimal `configuration.nix` for our distro might look like this:

```nix
{ config, pkgs, lib, ... }:
{
  # Do not edit the following system configuration for Asterinas NixOS
  nixpkgs.overlays = [ (import ./asterinas.nix) ];

  # Edit this list to add or remove installed software
  environment.systemPackages = with pkgs; [
    gcc
    python33
    vim
  ];
}
```

Here, the sample `configuration.nix` uses a NixOS feature called [overlays](https://wiki.nixos.org/wiki/Overlays) to customize and extend Nixpkgs without modifying its upstream source. Overlays apply globally within the configuration, so our Asterinas-specific changes to vanilla NixOS can be expressed cleanly as reusable overlays rather than as ad-hoc patches scattered throughout the system.

A further significant advantage of NixOS is the portability of [Nix](https://nixos.org/download/#nix-install-linux), its package manager (and the Nix language it uses), across various Linux distros and even macOS.

Unlike most package managers, which are tightly coupled to their parent distros (e.g., `pacman` for Arch, `dpkg` for Debian, `rpm` for Red Hat-based systems), Nix operates independently. Nix packages are installed in the `/nix/store` directory, ensuring they do not conflict with native packages. This portability is invaluable for debugging: if a package in our NixOS-based distro encounters an issue, we can replicate the exact same package environment on any Linux development machine, greatly simplifying troubleshooting.

### The New Top-Level Sub-Project

Given these compelling rationales, we propose the creation of a new top-level sub-project named _Asterinas NixOS (AsterNixOS)_. This sub-project will reside in a new `distro/` directory at the project root.

Initially, AsterNixOS will share version numbers with the Asterinas kernel, emphasizing its early development stage and close coupling. Every kernel release will be accompanied by a compatible distro release. In the long term, once both the kernel and the distro achieve maturity and stability, AsterNixOS will adopt its own independent versioning scheme, likely following a "YY.MM" format (e.g., 25.12).

## Drawbacks, Alternatives, and Unknowns

### Drawbacks

* **Learning curve for Nix/NixOS:** This is arguably the most significant hurdle. While exceptionally powerful, the Nix ecosystem, including the Nix language and its functional paradigm, presents a steeper learning curve compared to conventional package managers and build systems. This could pose an initial barrier for new contributors and existing Asterinas developers unfamiliar with Nix. Investing in clear documentation and onboarding resources will be critical.
* **Maintenance overhead of Nixpkgs overlay:** While the override pattern offers flexibility, maintaining a dedicated overlay for Asterinas-specific patches and configurations within the vast `nixpkgs` repository will still require continuous effort. Keeping pace with upstream `nixpkgs` changes, resolving potential conflicts, and ensuring compatibility will be an ongoing challenge requiring dedicated resources.

### Alternatives

* **Embedded Linux distros:** [Buildroot](https://buildroot.org/) and [Yocto](https://www.yoctoproject.org/) are established and highly capable tools for building embedded Linux distros, offering extensive customization for toolchains and root filesystems from scratch.
    * **Why NixOS is superior:** While powerful for embedded use cases, Buildroot and Yocto are less oriented toward a general-purpose desktop/server distro, which is our primary initial target. Furthermore, their configuration languages are generally less expressive and composable than the Nix language, limiting the flexibility we seek for deep customization and elegant overriding.
* **Direct port of Debian/Arch/etc.:** This approach would involve directly modifying the build systems of a more traditional distro like Debian or Arch to target Asterinas.
    * **Why NixOS is superior:** As elaborated, traditional packaging systems are less flexible for the granular, targeted patching and configuration required when adapting to a new kernel. More critically, their package managers (e.g., `dpkg`, `pacman`, `rpm`) are inherently tied to their parent distros, making cross-distro development and debugging significantly more complex. The Nix package manager's portability offers a clear advantage here.

## Prior Art and References

The NixOS project itself serves as the most prominent piece of prior art, demonstrating the viability and advantages of a purely functional approach to OS configuration and package management. Its design principles, particularly around atomic upgrades, rollbacks, and reproducible builds, have been thoroughly proven since its inception in 2003. Other projects, like [GNU Guix](https://guix.gnu.org/), also follow a similar functional package management paradigm.
