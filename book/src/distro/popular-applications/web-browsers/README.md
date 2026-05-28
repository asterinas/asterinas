# Web Browsers

This category covers web browsers.

## Browsers

### Firefox

[Firefox](https://www.mozilla.org/firefox/) is a graphical web browser.

#### Installation

```nix
# Enable the X11 (X.Org) desktop (XFCE)
services.xserver.enable = true;
services.xserver.desktopManager.xfce.enable = true;

# Enable the Firefox web browser
programs.firefox.enable = true;
```

#### Verified Usage

```bash
# Open a website in a new window
export DISPLAY=:0
firefox --new-instance https://example.com

# Take a screenshot of a website
firefox --headless --screenshot https://example.com
```

### Links2

[Links2](http://links.twibright.com/) is a text and graphics web browser.

#### Installation

```nix
environment.systemPackages = [ pkgs.links2 ];
```

#### Verified Usage

```bash
# Open a website
links http://example.com

# Dump page contents as text
links -dump http://example.com
```

### w3m

[w3m](https://w3m.sourceforge.net/) is a text-based web browser.

#### Installation

```nix
environment.systemPackages = [ pkgs.w3m ];
```

#### Verified Usage

```bash
# Open a website in text mode
w3m http://example.com

# Dump page contents as text
w3m -dump http://example.com
```
