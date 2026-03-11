# Desktop Environments & Display

This category covers desktop environments, window managers, and display servers.

## Desktop Environments

### Xfce

[Xfce](https://www.xfce.org/) is a lightweight desktop environment for UNIX-like operating systems.

#### Installation

Add the following lines to the `configuration.nix` file:

```nix
services.xserver.enable = true;
services.xserver.desktopManager.xfce.enable = true;
```

<!--
TODO: upgrade mdbook to enable admonition blocks like the one below:

> [!WARNING]
> Xfce must be enabled during the initial installation of Asterinas NixOS. Applying configuration changes via `nixos-rebuild` is not working yet.
-->

#### Verified Backends

* Display server:
  * Xorg display server
* Graphics drivers:
  * Standard UEFI VGA framebuffer

#### Verified Functionality

* Changing desktop wallpapers and background settings
* Adjusting font size, style, and system theme
* Creating application shortcuts and desktop launchers
* Managing panels and window behavior
* Using the settings manager and file browser

#### Verified GUI Applications

Utilities:

* `galculator`: Calculator
* `mousepad`: The default Xfce text editor
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
* `xgalaga`: Galaga-style arcade game

### TODO: GNOME

[GNOME](https://www.gnome.org/) is a popular desktop environment focused on simplicity and ease of use.

### TODO: KDE Plasma

[KDE Plasma](https://kde.org/plasma-desktop/) is a feature-rich desktop environment with extensive customization options.

## Window Managers

### TODO: i3

[i3](https://i3wm.org/) is a tiling window manager designed for power users.

### TODO: Sway

[Sway](https://swaywm.org/) is a tiling Wayland compositor compatible with i3.

### TODO: Hyprland

[Hyprland](https://hyprland.org/) is a dynamic tiling Wayland compositor.

## Display Servers & Compositors

### Xorg

[Xorg](https://www.x.org/) is the X Window System display server.

### TODO: Wayland

[Wayland](https://wayland.freedesktop.org/) is a display server protocol intended to replace X11.
