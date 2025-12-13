# Desktop Environment

## Xfce

[Xfce](https://www.xfce.org/) is a lightweight desktop environment for UNIX-like operating systems.

### Installation

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

### Verified Backends

* Display server:
  * Xorg display server
* Graphics drivers:
  * Standard UEFI VGA framebuffer

### Verified Functionality

* Changing desktop wallpapers and background settings
* Adjusting font size, style, and system theme
* Creating application shortcuts and desktop launchers
* Managing panels and window behavior
* Using the settings manager and file browser

### Verified GUI Applications

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
