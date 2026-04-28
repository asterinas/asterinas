# Web Browsers

This category covers web browsers.

## Browsers

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

### W3m

[W3m](https://w3m.sourceforge.net/) is a text-based web browser.

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
