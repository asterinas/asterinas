{ stdenv
, fetchurl
, pkg-config
, python310
, automake
, autoconf
, perl
, libtool
, lib
, pkgs
, meson
, ninja
}:

let
  buildXorgComponent = import ./buildComponent.nix {
    inherit stdenv lib pkg-config python310 automake autoconf perl libtool;
  };

  xtransVersion = "1.4.0";
  xtrans = buildXorgComponent {
    pname = "xtrans";
    version = xtransVersion;
    src = pkgs.xorg.xtrans.src;
    extraBuildInputs = [ pkg-config pkgs.xorg.libXau ];
    argConfigurePhase = ''
      ./configure --prefix=$out
    '';
  };

  xcbprotoVersion = "1.14.1";
  xcbproto = buildXorgComponent {
    pname = "xcb-proto";
    version = xcbprotoVersion;
    src = pkgs.xcb-proto.src;
    extraBuildInputs = [ pkgs.xorg.libXau ];
  };

  xorgprotoVersion = "1.14.1";
  xorgproto = buildXorgComponent {
    pname = "xorgproto";
    version = pkgs.xorg.xorgproto.version;
    src = pkgs.xorg.xorgproto.src;
    extraBuildInputs = [
      pkg-config
      pkgs.xorg.utilmacros
    ];
  };

  libxcbVersion = "1.14";
  libxcb = buildXorgComponent {
    pname = "libxcb";
    version = libxcbVersion;
    src = pkgs.xorg.libxcb.src;
    extraBuildInputs = [ xcbproto pkgs.xorg.libXau ];
  };

  libevdevVersion = "1.12.1";
  libevdev = buildXorgComponent {
    pname = "libevdev";
    version = libevdevVersion;
    src = pkgs.libevdev.src;
    extraBuildInputs = [ pkg-config ];
  };

  libx11Version = "1.7.5";
  libx11 = buildXorgComponent {
    pname = "libx11";
    version = libx11Version;
    src = pkgs.xorg.libX11.src;
    extraBuildInputs = [ libxcb xtrans xorgproto pkgs.xorg.libXau];
  };

  xeyesVersion = "1.3.1";
  xeyes = buildXorgComponent {
    pname = "xeyes";
    version = xeyesVersion;
    src = pkgs.xorg.xeyes.src;
    extraBuildInputs = [
      libx11
      pkgs.xorg.libXt
      pkgs.xorg.libXext
      pkgs.xorg.libXi
      xorgproto
      pkgs.xorg.utilmacros
      pkgs.xorg.libXmu
      pkgs.xorg.libXrender
    ];
  };

  xorgServerVersion = "21.1.4";
  xorgServer = buildXorgComponent {
    pname = "xorg-server";
    version = xorgServerVersion;
    src = pkgs.xorg.xorgserver.src;
    patches = [
      ./patches/xorgServer/0001-Skip-checking-graphics-under-sys.patch
    ];
    extraBuildInputs = [
      meson
      ninja
      libx11
      libxcb
      xtrans
      xorgproto
      pkgs.xorg.utilmacros
      pkgs.xorg.libXau
      pkgs.xorg.libXdmcp
      pkgs.xorg.pixman
      pkgs.libdrm
      pkgs.mesa
      pkgs.mesa-demos
      pkgs.mesa-gl-headers
      pkgs.libGL
      pkgs.openssl
      pkgs.xorg.libxcvt
      pkgs.xorg.libXfont2
      pkgs.xorg.xdriinfo
      pkgs.xorg.libxkbfile
      pkgs.dri-pkgconfig-stub
      pkgs.xorg.libpciaccess
      pkgs.libepoxy
      pkgs.glamoroustoolkit
      pkgs.libgbm
      pkgs.xorg.xf86inputevdev
      pkgs.xorg.xf86videofbdev
      pkgs.fontconfig
      pkgs.dejavu_fonts
      pkgs.xkeyboard_config
      pkgs.xorg.fontsunmisc
      pkgs.xorg.fontutil
      pkgs.libudev-zero
      pkgs.libtirpc
    ];
    argConfigurePhase = ''
      meson setup builddir \
        --prefix=$out \
        --libdir=lib \
        -Dxorg=true \
        -Dglamor=true \
        -Dxkb_output_dir=/var/lib/xkb \
        -Doptimization=0 \
        -Ddebug=true \
        -Dudev=false \
        -Dxkb_bin_dir=/usr/bin \
        -Dudev_kms=false
    '';
    argBuildPhase = ''
      ninja -C builddir
    '';
    argInstallPhase = ''
      ninja -C builddir install
    '';
  };

  evtestVersion = "1.35";
  evtest = buildXorgComponent {
    pname = "evtest";
    version = evtestVersion;
    src = pkgs.evtest.src;
    extraBuildInputs = [
      pkgs.autoconf
      pkgs.automake
      pkgs.libtool
      pkgs.pkg-config
    ];
    argConfigurePhase = ''
      autoreconf -fiv
      ./configure --prefix=$out
    '';
    argBuildPhase = ''
      make
    '';
    argInstallPhase = ''
      make install
    '';
  };
in
{
  inherit xtrans xcbproto xorgproto libxcb libx11 xeyes libevdev xorgServer evtest;
}