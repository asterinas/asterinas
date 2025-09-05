{ stdenv
, fetchFromGitHub
, pkg-config
, gtk3
, wrapGAppsHook
, lib
, autoconf
, automake
, libtool
, pkgs
}:

let
  xorg = pkgs.callPackage ./xorg.nix {
    inherit pkgs;
  };

  buildComponent = import ./buildComponent.nix {
    inherit stdenv lib pkg-config automake autoconf libtool;
  };

in {
  xfwm4 = buildComponent {
    pname = "xfwm4";
    version = "4.16.1";
    src = pkgs.xfce.xfwm4.src;
    argNativeBuildInputs = [
      pkg-config
      wrapGAppsHook
      pkgs.xfce.xfce4-dev-tools
      autoconf
      automake
      libtool
    ];
    extraBuildInputs = [
      gtk3
      pkgs.xfce.libxfce4util
      pkgs.xfce.libxfce4ui
      pkgs.libwnck
      xorg.libxcb
      pkgs.xfce.xfce4-panel
      pkgs.xfce.xfconf
      pkgs.xfce.xfdesktop
      pkgs.xfce.thunar
      pkgs.xfce.exo
      pkgs.xfce.garcon
      pkgs.xfce.tumbler
      pkgs.librsvg
    ];
    argPreConfigure = ''./autogen.sh'';
    configureFlags = [
      "--enable-maintainer-mode"
      "--enable-debug=no"
    ];
    metaArgs = with lib; {
      description = "XFCE Component: xfwm4";
      homepage = "https://xfce.org";
      license = licenses.gpl2Plus;
      platforms = platforms.unix;
    };
  };

  xfdesktop = buildComponent {
    pname = "xfdesktop";
    version = "4.16.0";
    src = pkgs.xfce.xfdesktop.src;
    patches = [
      ./patches/xfdesktop4/0001-Fix-not-using-consistent-monitor-identifiers.patch
    ];
    argNativeBuildInputs = [
      pkg-config
      wrapGAppsHook
      pkgs.xfce.xfce4-dev-tools
      autoconf
      automake
      libtool
      pkgs.glib
      pkgs.gettext
    ];
    extraBuildInputs = [
      gtk3
      pkgs.xfce.libxfce4util
      pkgs.xfce.libxfce4ui
      pkgs.xfce.xfconf
      pkgs.xfce.exo
      pkgs.xfce.garcon
      pkgs.xfce.tumbler
      pkgs.librsvg
      pkgs.libwnck
      pkgs.glib
      pkgs.xfce.libxfce4windowing
      pkgs.hicolor-icon-theme
      pkgs.xfce.xfce4-panel
      pkgs.adwaita-icon-theme
      pkgs.gtk3
      pkgs.gdk-pixbuf
      pkgs.libjpeg                  # JPEG decoder
      pkgs.libpng                   # PNG support (good to have)
      pkgs.libtiff                  # TIFF support
      pkgs.gdk-pixbuf-xlib          # Extended pixbuf support
      pkgs.shared-mime-info         # MIME type detection
      pkgs.libyaml
    ];
    configureFlags = [
      "--enable-maintainer-mode"
      "--enable-debug=no"
      "--enable-desktop-icons"
      "--enable-file-icons"
      "--with-svg"                  # Enable SVG support
      "--with-jpeg"                 # Enable JPEG support
    ];
    argPreConfigure = ''./autogen.sh'';
    metaArgs = with lib; {
      description = "XFCE Component: xfdesktop4";
      homepage = "https://xfce.org";
      license = licenses.gpl2Plus;
      platforms = platforms.unix;
    };
  };
}