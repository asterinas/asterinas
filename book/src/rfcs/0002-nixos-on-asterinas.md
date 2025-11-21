# RFC-0002: NixOS on Asterinas

* Status: Draft
* Pull request: https://github.com/asterinas/asterinas/pull/2584
* Date submitted: 2025-11-14
* Date approved: YYYY-MM-DD

## Summary

This RFC formally proposes the establishment of an Asterinas distribution as a new, top-level sub-project of Asterinas. We intend for this distribution to leverage [NixOS](https://nixos.org/) due to its unparalleled customizability and rich package ecosystem. Accordingly, we propose naming this new sub-project **NixOS on Asterinas (NOSA)**.

## Motivations

### What is a "distro"?

In the context of operating systems (OSes), a "distribution" (or "distro") typically refers to a complete OS built around a kernel, complemented by userspace tools, libraries, and applications managed via a package management system. A Linux distro, for example, combines the Linux kernel with a rich userspace. Similarly, an Asterinas distro will pair the Asterinas kernel with a comprehensive userspace environment. For the purpose of this discussion, "distro" will refer broadly to either a Linux or Asterinas-based distro.

### Short-term strategic advantages

Achieving a Minimal Viable Product (MVP) milestone is crucial for Asterinas's maturation. Reaching MVP means that Asterinas is ready for evaluation by early adopters, who expect a seamless experience comparable to mainstream OSes. This includes easy application installation and out-of-the-box functionality. Simply providing a kernel is insufficient; we must deliver a user-friendly experience, which necessitates a full-fledged distro complete with an intuitive package manager.

Furthermore, direct control over an Asterinas distro offers a pragmatic approach to addressing Linux compatibility challenges. While Asterinas is committed to a high degree of Linux compatibility, achieving perfect parity in the near term is impractical. A dedicated distro allows us to configure or patch specific packages to circumvent reliance on advanced or bleeding-edge Linux features that do not have high priorities in Asterinas. This significantly reduces pressure on Asterinas kernel developers to implement complex features prematurely, enabling a more focused and stable development roadmap.

Beyond external users, Asterinas developers themselves will greatly benefit from a dedicated distro. Testing complex applications with intricate dependencies is a significant hurdle. While "Hello World" or even medium-sized projects like Nginx or Redis can be manually built and integrated into a disk image, this approach becomes unscalable for large-scale applications such as Spark, PyTorch, or Chromium. Identifying, understanding, and building every dependency for such projects would divert critical kernel development resources. A distro, by its very nature, abstracts this complexity through its package manager, streamlining the testing and development workflow for all Asterinas developers.

### Long-term ecosystem vision

Looking ahead, we envision a thriving ecosystem of Asterinas distros. This initial, kernel-developer-maintained distro will serve as a vital reference implementation, fostering the creation of diverse Asterinas-based distros. While the Linux world boasts numerous distros, Asterinas-specific distros will be essential for two primary reasons:

Firstly, Asterinas prioritizes Linux compatibility at the external ABI level, not the internal kernel module interface. This means Asterinas cannot load Linux kernel modules (`.ko` files). An Asterinas distro will ensure that all system programs exclusively load Asterinas kernel modules, maintaining integrity and security within our native environment.

Secondly, Asterinas will eventually offer unique features and differentiated value propositions compared to Linux. These may include novel system calls, file systems, or device drivers. Realizing the full potential and value of these unique features will inherently require cooperation from the userspace—that is, the distro itself.

In essence, establishing Asterinas distros is paramount for Asterinas's long-term success and differentiation. This RFC lays the groundwork for that future by proposing our foundational distro.

## Design

### Why NixOS as the base distro?

Building the first Asterinas distro from scratch would be an immense undertaking, diverting focus from core kernel development. Therefore, basing it on an existing Linux distro is the most rational path forward. This leads to the crucial question: which Linux distro should serve as our foundation?

The landscape of Linux distros is vast, featuring prominent names such as [Arch](https://archlinux.org/), [CentOS](https://www.centos.org/), [Debian](https://www.debian.org/), and [Gentoo](https://www.gentoo.org/). While theoretically, any of these could serve, we have identified [NixOS](https://nixos.org/) as a particularly attractive and uniquely suited option.

In most distros, package recipes are centered around shell scripts (e.g., [`PKGBUILD`](https://man.archlinux.org/man/PKGBUILD.5#EXAMPLE) in Arch Linux or the [`rules`](https://www.debian.org/doc/manuals/maint-guide/dreq.en.html#rules) file in a Debian package). NixOS, however, utilizes a purpose-built, purely functional language called [Nix](https://nixos-and-flakes.thiscute.world/the-nix-language/), which offers unparalleled expressiveness and flexibility for defining package recipes.

A core requirement for our base distro is the ability to easily tweak existing package recipes to work around Asterinas limitations or customize the new distro's look or behavior. The Nix language excels in this regard.

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

Another powerful example is the ease with which we can create an Asterinas distro ISO installer by customizing an existing [NixOS installer ISO](https://nixos.org/download/#nixos-iso) through a modified `configuration.nix` file:

```nix
{ config, lib, pkgs, ... }: {
  options = {
    asterinas.splash = lib.mkOption {
      type = lib.types.path;
      default = /asterinas/splash.png;
    };
    asterinas.kernel = lib.mkOption {
      type = lib.types.path;
      default = /asterinas/kernel;
    };
    asterinas.initramfs = lib.mkOption {
      type = lib.types.path;
      default = pkgs.makeInitrd {
        compressor = "gzip";
        contents = [
          {
            object = "${pkgs.busybox}/bin";
            symlink = "/bin";
          }
          {
            object = "${config.asterinas.initramfs-init}";
            symlink = "/init";
          }
        ];
      };
    };
  };

  config = {
    boot.loader.grub.enable = true;
    boot.loader.grub.efiSupport = true;
    boot.loader.grub.device = "nodev";
    boot.loader.grub.efiInstallAsRemovable = true;
    boot.loader.grub.splashImage = config.asterinas.splash;

    boot.initrd.enable = false;
    boot.kernel.enable = false;
    boot.loader.grub.extraInstallCommands = ''
      echo "Executing more commands after grub installation.."
    '';
    boot.postBootCommands = ''
      echo "Executing more commands after booting.."
    '';

    system.systemBuilderCommands = ''
      echo "Building the kernel and initrd.."
    '';
    systemd.enableCgroupAccounting = false;

    environment.defaultPackages = [ ];

    system.nixos.distroName = "Asterinas";
  };
}
```

This configuration demonstrates how to modify some aspects of the NixOS installer ISO, such as the kernel, initramfs, and GRUB configuration, without needing to fork the entire NixOS build system. This approach minimizes the amount of project-specific code we need to maintain for our NixOS-based Asterinas distro.

A further significant advantage of NixOS is the portability of [Nix](https://nixos.org/download/#nix-install-linux), its package manager (not Nix, the programming language), across various Linux distros and even macOS.

Unlike most package managers, which are tightly coupled to their parent distros (e.g., Pacman for Arch, `dpkg` for Debian, `rpm` for Red Hat-based systems), Nix operates independently. Nix packages are installed in the `/nix/store` directory, ensuring they do not conflict with native packages. This portability is invaluable for debugging: if a package in our NixOS-based distro encounters an issue, we can replicate the exact same package environment on any Linux development machine, greatly simplifying troubleshooting.

### The New Top-Level Sub-Project

Given these compelling rationales, we propose the creation of a new top-level sub-project named _NixOS on Asterinas (NOSA)_. This sub-project will reside in a new `distro/` directory at the project root.

Initially, NOSA will share version numbers with the Asterinas kernel, emphasizing its early development stage and close coupling. Every kernel release will be accompanied by a compatible distro release. In the long term, once both the kernel and the distro achieve maturity and stability, NOSA will adopt its own independent versioning scheme, likely following a "YY.MM" format (e.g., 25.12).

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
