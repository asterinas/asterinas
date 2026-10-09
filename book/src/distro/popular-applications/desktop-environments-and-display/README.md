# Desktop Environments & Display

This category covers desktop environments, window managers, and display servers.

## Desktop Environments

### Xfce

[Xfce](https://www.xfce.org/) is a lightweight desktop environment for UNIX-like operating systems.

#### Installation

To enable the Xfce desktop, add the following settings to `configuration.nix` before building the installation image:

```nix
hardware.graphics.enable = true;
services.xserver.enable = true;
services.xserver.desktopManager.xfce.enable = true;
```

**Note:** Enable Xfce during the initial installation of Asterinas NixOS. Applying configuration changes through `nixos-rebuild` is not supported yet.

To include additional [verified GUI applications](#verified-gui-applications), add their package names to `environment.systemPackages` in the same file.

For example, to install galculator:

```nix
environment.systemPackages = with pkgs; [
  # Add packages from the applications listed below.
  galculator
];
```

#### Improving Desktop Responsiveness

<!--
TODO: Revisit this guidance when hardware-accelerated GPU drivers are supported.
-->

The current graphics stack relies on the CPU for rendering and display updates.

Giving the VM more virtual CPUs (vCPUs) can improve desktop responsiveness by allowing graphics work and other tasks to run concurrently.

For end users, add `-smp 4` to the QEMU boot command in the [Getting Started guide](../../#end-users).

For example, replace its CPU and memory options with:

```bash
-cpu host -smp 4 -m 8G -enable-kvm \
```

For kernel developers, set `SMP` (symmetric multiprocessing) to the desired vCPU count when starting the VM:

```bash
make run_nixos SMP=4
```

#### Verified Backends

* Display server:
  * Xorg display server with the `modesetting` driver over DRM/KMS
* Graphics stack:
  * `simpledrm` over the standard UEFI framebuffer
  * Mesa software rendering through GLX

#### Verified Functionality

* Changing desktop wallpapers and background settings
* Adjusting font size, style, and system theme
* Creating application shortcuts and desktop launchers
* Managing panels and window behavior
* Using the settings manager and file browser

#### Verified GUI Applications

After starting the Xfce desktop, find installed applications in the Applications menu and click an application's entry to launch it.

Included with Xfce (no separate entry in `environment.systemPackages` is needed):

* `mousepad`: Text editor

Utilities:

* `galculator`: Calculator
* `mupdf`: A lightweight PDF and XPS viewer

Games:

* `fairymax`: Chess
* `five-or-more`: GNOME alignment game
* `lbreakout2`: Breakout/Arkanoid clone
* `gnome-chess`: GNOME chess
* `gnome-mines`: Minesweeper
* `gnome-sudoku`: GNOME sudoku
* `tali`: GNOME dice game
* `xboard`: Chess

3D Games:

* `openarena`: First-person arena shooter
* `supertuxkart`: Kart racing game
* `neverball`: Tilt-controlled ball rolling game
