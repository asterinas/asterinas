{ lib, stdenvNoCC, fetchFromGitHub, hostPlatform, writeClosure, busybox, apps
, benchmark, syscall, xfce ? null, xorg ? null, pkgs }:

let
  etc = lib.fileset.toSource {
    root = ./../src/etc;
    fileset = ./../src/etc;
  };

  gvisor_libs = builtins.path {
    name = "gvisor-libs";
    path = "/lib/x86_64-linux-gnu";
  };

  all_pkgs = [ busybox etc ] ++ lib.optionals (apps != null) [ apps.package ]
    ++ lib.optionals (benchmark != null) [ benchmark.package ]
    ++ lib.optionals (syscall != null) [ syscall.package ]
    ++ lib.optionals (xfce != null) [
         xfce.xfwm4
         xfce.xfdesktop
         pkgs.xfce.thunar
         pkgs.xfce.xfce4-panel
         pkgs.xfce.xfce4-terminal
         pkgs.xfce.mousepad
         pkgs.xfce.xfce4-appfinder
         pkgs.xfce.xfce4-settings
         pkgs.xfce.tumbler
       ]
    ++ lib.optionals (xorg != null) [
         xorg.xtrans
         xorg.libxcb
         xorg.xcbproto
         xorg.libx11
         pkgs.xorg.xeyes
         xorg.libevdev
         xorg.xorgServer
         pkgs.xorg.xf86videofbdev
         pkgs.xorg.libxkbfile
         pkgs.xorg.xf86inputevdev
         pkgs.dbus
         pkgs.hicolor-icon-theme
         pkgs.evtest
         pkgs.adwaita-icon-theme
         pkgs.gdk-pixbuf
         pkgs.gdk-pixbuf.dev
         pkgs.gdk-pixbuf-xlib
         pkgs.librsvg
         pkgs.libjpeg
         pkgs.libpng
         pkgs.shared-mime-info
         pkgs.dconf
         pkgs.gsettings-desktop-schemas
         pkgs.glib
         pkgs.glib.bin
         pkgs.glib-networking

         # GNOME Games
         pkgs.gnome-mines
         pkgs.gnome-sudoku
         pkgs.five-or-more
         pkgs.tali
         pkgs.gnome-chess
       ];
in stdenvNoCC.mkDerivation {
  name = "initramfs";
  buildCommand = ''
    set -euo pipefail

    # Function to recursively create directory structure
    create_directory_structure() {
      local source_package="$1"
      local dest_base="$2"

      echo "=== Creating directory structure ==="
      echo "Source: $source_package"
      echo "Destination: $dest_base"
      echo ""

      # Recursive function to process directories
      process_directory() {
        local current_source="$1"
        local current_dest="$2"
        local depth="$3"

        # Create indentation for tree-like output
        local indent=""
        for ((i=0; i<depth; i++)); do
          indent="$indent  "
        done

        # Check if destination directory exists, create if it doesn't
        if [ ! -d "$current_dest" ]; then
          echo "$indent Creating: $current_dest"
          mkdir -p "$current_dest"
        else
          echo "$indent Exists: $current_dest"
        fi

        # Scan immediate subdirectories and process them recursively
        if [ -d "$current_source" ]; then
          find "$current_source" -maxdepth 1 -type d ! -path "$current_source" | sort | while read -r subdir; do
            local subdir_name=$(basename "$subdir")
            local dest_subdir="$current_dest/$subdir_name"

            echo "$indent ├── Processing: $subdir_name/"

            # Recursively process this subdirectory
            process_directory "$subdir" "$dest_subdir" $((depth + 1))
          done
        fi
      }

      # Start the recursive process
      process_directory "$source_package" "$dest_base" 0

      echo ""
      echo "=== Directory structure creation completed ==="
    }

    # Function to copy all files while preserving directory structure
    copy_with_structure() {
      local source_package="$1"
      local dest_base="$2"

      echo "Copying files from $source_package to $dest_base"

      # First create all directories
      create_directory_structure "$source_package" "$dest_base"

      # Then copy all files
      find "$source_package" -type f | while read -r source_file; do
        # Get the relative path
        relative_path="''${source_file#$source_package}"
        relative_path="''${relative_path#/}"

        if [ -n "$relative_path" ]; then
          dest_file="$dest_base/$relative_path"
          dest_dir="$(dirname "$dest_file")"

          # Ensure destination directory exists
          mkdir -p "$dest_dir"

          # Copy the file
          echo "Copying: $source_file -> $dest_file"
          cp -af "$source_file" "$dest_file"
        fi
      done
    }

    # Function to process package directories with custom mappings
    process_package_mappings() {
      local package="$1"
      local mappings="$2"
      local package_name="$3"

      echo "=== Processing $package_name ==="

      for mapping in $mappings; do
        source_subdir=$(echo "$mapping" | cut -d':' -f1)
        dest_base=$(echo "$mapping" | cut -d':' -f2)

        package_source="$package/$source_subdir"

        if [ -d "$package_source" ]; then
          echo "  $source_subdir -> $dest_base"
          create_directory_structure "$package_source" "$dest_base"
          copy_with_structure "$package_source" "$dest_base"
        else
          echo "  Skipping $source_subdir (not found)"
        fi
      done

      echo "=== $package_name processing completed ==="
      echo ""
    }

    # Create base directory structure
    mkdir -p $out/{dev,etc,root,usr,opt,tmp,var,proc,sys}
    mkdir -p $out/{benchmark,test,ext2,exfat}
    mkdir -p $out/usr/{bin,sbin,lib,lib64,local}

    # Create symbolic links
    ln -sfn usr/bin $out/bin
    ln -sfn usr/sbin $out/sbin
    ln -sfn usr/lib $out/lib
    ln -sfn usr/lib64 $out/lib64

    # Copy busybox
    cp -r ${busybox}/bin/* $out/bin/

    # Copies the contents of the /etc
    cp -r ${etc}/* $out/etc/

    ${lib.optionalString (xfce != null) ''
      # XFConf
      xfconf_mappings="bin:$out/usr/bin share:$out/usr/share"
      process_package_mappings "${pkgs.xfce.xfconf}" "$xfconf_mappings" "XFConf"
      cp ${pkgs.xfce.xfconf}/lib/xfce4/xfconf/xfconfd $out/usr/bin

      # XFWM4 Window Manager
      xfwm4_mappings="bin:$out/usr/bin share:$out/usr/share"
      process_package_mappings "${xfce.xfwm4}" "$xfwm4_mappings" "XFWM4"

      # XFDesktop
      xfdesktop_mappings="bin:$out/usr/bin share:$out/usr/share"
      process_package_mappings "${xfce.xfdesktop}" "$xfdesktop_mappings" "XFDesktop"

      # Generate xfce4-desktop.xml with default wallpaper settings
      mkdir -p $out/etc/xdg/xfce4/xfconf/xfce-perchannel-xml
      cat > $out/etc/xdg/xfce4/xfconf/xfce-perchannel-xml/xfce4-desktop.xml << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<channel name="xfce4-desktop" version="1.0">
  <property name="last-settings-migration-version" type="uint" value="1"/>
  <property name="backdrop" type="empty">
    <property name="screen0" type="empty">
      <property name="monitordefault" type="empty">
        <property name="workspace0" type="empty">
          <property name="last-image" type="string" value="/usr/share/backgrounds/xfce/xfce-flower.svg"/>
        </property>
      </property>
    </property>
  </property>
  <property name="last" type="empty">
    <property name="window-width" type="int" value="708"/>
    <property name="window-height" type="int" value="547"/>
  </property>
</channel>
EOF

      # XFCE4 Panel
      panel_mappings="bin:$out/usr/bin etc:$out/etc share:$out/usr/share"
      process_package_mappings "${pkgs.xfce.xfce4-panel}" "$panel_mappings" "XFCE4-Panel"

      # Dconf (GSettings backend + dconf-service)
      dconf_mappings="bin:$out/usr/bin etc:$out/etc lib:$out/usr/lib libexec:$out/usr/libexec share:$out/usr/share"
      process_package_mappings "${pkgs.dconf}" "$dconf_mappings" "Dconf"
      cp -L ${pkgs.dconf.lib}/libexec/dconf-service $out/usr/bin
      # Also copy dconf’s libraries (contains gio module: libdconfsettings.so)
      dconf_lib_mappings="lib:$out/usr/lib"
      process_package_mappings "${pkgs.dconf.lib}" "$dconf_lib_mappings" "Dconf-Libs"

      # GSettings schemas (needed by many GTK apps)
      schemas_dst="$out/usr/share/glib-2.0/schemas"
      mkdir -p "$schemas_dst"
      # Copy from plain path if present
      if [ -d "${pkgs.gsettings-desktop-schemas}/share/glib-2.0/schemas" ]; then
        find "${pkgs.gsettings-desktop-schemas}/share/glib-2.0/schemas" -maxdepth 1 -type f -name '*.gschema.xml' -exec cp -af {} "$schemas_dst"/ \;
      fi
      # Copy from Nix’s versioned gsettings-schemas path
      for d in ${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/*/glib-2.0/schemas; do
        if [ -d "$d" ]; then
          find "$d" -maxdepth 1 -type f -name '*.gschema.xml' -exec cp -af {} "$schemas_dst"/ \;
        fi
      done

      # GLib tools (gdbus, gsettings, glib-compile-schemas)
      glib_tools_mappings="bin:$out/usr/bin"
      process_package_mappings "${pkgs.glib.bin}" "$glib_tools_mappings" "GLib-Tools"

      glib_tools_dev_mappings="bin:$out/usr/bin"
      process_package_mappings "${pkgs.glib.dev}" "$glib_tools_dev_mappings" "GLib-Tools-Dev"

      # Compile schemas for runtime
      ${pkgs.glib.dev}/bin/glib-compile-schemas "$schemas_dst"

      # GIO modules (dconf backend + TLS/proxy from glib-networking)
      glib_networking_mappings="lib:$out/usr/lib share:$out/usr/share"
      process_package_mappings "${pkgs.glib-networking}" "$glib_networking_mappings" "glib-networking"

      # Create GIO module cache so loaders are discovered fast
      mkdir -p $out/usr/lib/gio/modules
      rm -rf $out/usr/lib/gio/modules/giomodule.cache
      ${pkgs.glib.dev}/bin/gio-querymodules $out/usr/lib/gio/modules \
        > $out/usr/lib/gio/modules/giomodule.cache \
        2> $out/usr/lib/gio/modules/giomodule.cache.log || true

      # Tumbler (thumbnailer)
      tumbler_mappings="bin:$out/usr/bin lib:$out/usr/lib libexec:$out/usr/libexec share:$out/usr/share"
      process_package_mappings "${pkgs.xfce.tumbler}" "$tumbler_mappings" "Tumbler"

      # Thunar File Manager
      thunar_mappings="bin:$out/usr/bin etc:$out/etc share:$out/usr/share"
      process_package_mappings "${pkgs.xfce.thunar}" "$thunar_mappings" "Thunar"

      # Configure Thunar as default file manager
      mkdir -p $out/usr/share/applications
      cat > $out/usr/share/applications/mimeapps.list << 'EOF'
[Default Applications]
inode/directory=thunar.desktop
application/x-directory=thunar.desktop
x-directory/normal=thunar.desktop

[Added Associations]
inode/directory=thunar.desktop;
application/x-directory=thunar.desktop;
EOF

      # Also create system-wide associations
      mkdir -p $out/etc/xdg
      cp $out/usr/share/applications/mimeapps.list $out/etc/xdg/mimeapps.list

      # XFCE4 Settings Manager
      settings_mappings="bin:$out/usr/bin etc:$out/etc share:$out/usr/share"
      process_package_mappings "${pkgs.xfce.xfce4-settings}" "$settings_mappings" "XFCE4-Settings"

      # XFCE4 Terminal
      terminal_mappings="bin:$out/usr/bin share:$out/usr/share"
      process_package_mappings "${pkgs.xfce.xfce4-terminal}" "$terminal_mappings" "XFCE4-Terminal"

      # Mousepad Text Editor
      mousepad_mappings="bin:$out/usr/bin share:$out/usr/share"
      process_package_mappings "${pkgs.xfce.mousepad}" "$mousepad_mappings" "Mousepad"

      # XFCE4 Application Finder
      appfinder_mappings="bin:$out/usr/bin share:$out/usr/share"
      process_package_mappings "${pkgs.xfce.xfce4-appfinder}" "$appfinder_mappings" "XFCE4-AppFinder"

      # Install GNOME Games

      # GNOME Mines (Minesweeper)
      mines_mappings="bin:$out/usr/bin share:$out/usr/share"
      process_package_mappings "${pkgs.gnome-mines}" "$mines_mappings" "GNOME-Mines"

      # GNOME Sudoku
      sudoku_mappings="bin:$out/usr/bin share:$out/usr/share"
      process_package_mappings "${pkgs.gnome-sudoku}" "$sudoku_mappings" "GNOME-Sudoku"

      # Five or More (Lines game)
      fiveormore_mappings="bin:$out/usr/bin share:$out/usr/share"
      process_package_mappings "${pkgs.five-or-more}" "$fiveormore_mappings" "Five-or-More"

      # Tali (Yahtzee-like dice game)
      tali_mappings="bin:$out/usr/bin share:$out/usr/share"
      process_package_mappings "${pkgs.tali}" "$tali_mappings" "Tali"

      # GNOME Chess
      chess_mappings="bin:$out/usr/bin share:$out/usr/share"
      process_package_mappings "${pkgs.gnome-chess}" "$chess_mappings" "GNOME-Chess"

      # Install Adwaita Icon Theme
      cp -raf ${pkgs.adwaita-icon-theme}/share/icons/Adwaita $out/usr/share/icons
      # Create icon theme configuration
      mkdir -p $out/usr/share/icons/default/
      cat > $out/usr/share/icons/default/index.theme << 'EOF'
[Icon Theme]
Name=Default
Comment=Default icon theme
Inherits=Adwaita,hicolor
Directories=.
EOF
      mkdir -p $out/etc/gtk-3.0
      cat > $out/etc/gtk-3.0/settings.ini << 'EOF'
[Settings]
gtk-icon-theme-name=Adwaita
gtk-theme-name=Adwaita
gtk-font-name=Sans 10
gtk-cursor-theme-name=Adwaita
gtk-cursor-theme-size=24
gtk-toolbar-style=GTK_TOOLBAR_BOTH_HORIZ
gtk-toolbar-icon-size=GTK_ICON_SIZE_LARGE_TOOLBAR
gtk-button-images=1
gtk-menu-images=1
gtk-enable-event-sounds=1
gtk-enable-input-feedback-sounds=1
gtk-xft-antialias=1
gtk-xft-hinting=1
gtk-xft-hintstyle=hintfull
EOF
      # Install GDK-Pixbuf utilities
      gdkpixbuf_mappings="bin:$out/usr/bin share:$out/usr/share"
      process_package_mappings "${pkgs.gdk-pixbuf}" "$gdkpixbuf_mappings" "GDK-Pixbuf"

      # Copy actual GDK-Pixbuf loader modules to initramfs paths
      mkdir -p $out/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders
      cp -af ${pkgs.gdk-pixbuf}/lib/gdk-pixbuf-2.0/2.10.0/loaders/* \
        $out/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders/ 2>/dev/null || true
      cp -af ${pkgs.librsvg}/lib/gdk-pixbuf-2.0/2.10.0/loaders/* \
        $out/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders/ 2>/dev/null || true

      # Generate loaders.cache for the /usr path inside the image
      cp -arf ${pkgs.librsvg}/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache $out/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache

      # Setup font cache directory
      mkdir -p $out/var/cache/fontconfig
      chmod 755 $out/var/cache/fontconfig

      # Install GDK-Pixbuf development tools (includes gdk-pixbuf-pixdata)
      gdkpixbufdev_mappings="bin:$out/usr/bin include:$out/usr/include lib:$out/usr/lib"
      process_package_mappings "${pkgs.gdk-pixbuf.dev}" "$gdkpixbufdev_mappings" "GDK-Pixbuf-Dev"

      # Install MIME
      mime_mappings="bin:$out/usr/bin share:$out/usr/share"
      process_package_mappings "${pkgs.shared-mime-info}" "$mime_mappings" "Shared-MIME-Info"

        ${lib.optionalString (xorg != null) ''
          # Install evtest
          cp ${pkgs.evtest}/bin/evtest $out/bin/

          # Install X.Org Server
          xorg_mappings="bin:$out/usr/bin lib:$out/usr/lib include:$out/usr/include"
          process_package_mappings "${xorg.xorgServer}" "$xorg_mappings" "X.Org-Server"

          # Install X.Org video driver
          xf86videofbdev_mappings="lib:$out/usr/lib"
          process_package_mappings "${pkgs.xorg.xf86videofbdev}" "$xf86videofbdev_mappings" "X.Org-FBDev-Driver"

          # Install X.Org input driver
          xf86inputevdev_mappings="lib:$out/usr/lib"
          process_package_mappings "${pkgs.xorg.xf86inputevdev}" "$xf86inputevdev_mappings" "X.Org-Input-Driver"

          # Copy X.Org configuration
          mkdir -p $out/usr/share/X11/xorg.conf.d/
          cp ${./patches/xorgServer/10-fbdev.conf} $out/usr/share/X11/xorg.conf.d/10-fbdev.conf

          # Install scripts
          cp ${./scripts/run_as_xfce.sh} $out/usr/bin/run_as_xfce.sh

          # Install fonts
          fontconfig_mappings="bin:$out/usr/bin etc:$out/etc share:$out/usr/share"
          process_package_mappings "${pkgs.fontconfig}" "$fontconfig_mappings" "FontConfig"

          dejavu_mappings="share:$out/usr/share"
          process_package_mappings "${pkgs.dejavu_fonts}" "$dejavu_mappings" "DejaVu-Fonts"

          fontsunmisc_mappings="lib:$out/usr/share/fonts"
          process_package_mappings "${pkgs.xorg.fontsunmisc}" "$fontsunmisc_mappings" "X11-Misc-Fonts"

          xkeyboard_mappings="share:$out/usr/share"
          process_package_mappings "${pkgs.xkeyboard_config}" "$xkeyboard_mappings" "XKeyboard-Config"

          xkbcomp_mappings="bin:$out/usr/bin"
          process_package_mappings "${pkgs.xorg.xkbcomp}" "$xkbcomp_mappings" "XKBComp"

          # Copy custom fonts configuration
          mkdir -p $out/etc/fonts/
          cp ${./patches/fonts/fonts.conf} $out/etc/fonts/fonts.conf

          # Install X.Org client applications
          cp -L ${pkgs.xorg.xeyes}/bin/* $out/usr/bin/

          # D-Bus specific setup
          mkdir -p $out/run/dbus
          chmod a+w $out/run/dbus -R
          mkdir -p $out/usr/local/share/dbus-1/system-services
          cp -raf ${pkgs.dbus}/bin/* $out/usr/bin/
          mkdir -p $out/etc/dbus-1
          cp -raf ${pkgs.dbus}/etc/dbus-1 $out/etc/dbus-1
          cp ${./patches/dbus/session.conf} $out/etc/dbus-1/session.conf
          cp ${./patches/dbus/system.conf} $out/etc/dbus-1/system.conf
          cp -raf ${pkgs.dbus}/etc/systemd $out/etc/systemd
          mkdir -p $out/usr/libexec/
          cp -raf ${pkgs.dbus}/libexec/* $out/usr/libexec/
          cp -raf ${pkgs.dbus}/lib/tmpfiles.d/* $out/usr/lib/tmpfiles.d
          cp -raf ${pkgs.dbus}/lib/sysusers.d/* $out/usr/lib/sysusers.d
          mkdir -p $out/usr/share/dbus-1
          cp -raf ${pkgs.dbus}/share/dbus-1/* $out/usr/share/dbus-1
          cp -raf ${pkgs.dbus}/share/xml/* $out/usr/share/xml
          mkdir -p $out/var/lib/dbus
          echo "52e0ad0e9794402c90315dd6af205511" > $out/var/lib/dbus/machine-id
          echo "52e0ad0e9794402c90315dd6af205511" > $out/etc/machine-id
          mkdir -p $out/run/current-system/sw/bin/
          ln -s ${pkgs.dbus}/bin/dbus-daemon $out/run/current-system/sw/bin/dbus-daemon

          # Install Hicolor Icon Theme
          hicolor_base="$out/usr/share/icons/hicolor"
          sizes="128x128 128x128@2 16x16 16x16@2 192x192 192x192@2 22x22 22x22@2 24x24 24x24@2 256x256 256x256@2 32x32 32x32@2 36x36 36x36@2 48x48 48x48@2 512x512 512x512@2 64x64 64x64@2 72x72 72x72@2 96x96 96x96@2 scalable"
          subdirs="actions animations apps categories devices emblems emotes filesystems intl mimetypes places status stock"
          stock_subdirs="chart code data form image io media navigation net object table text"

          for size in $sizes; do
            for sub in $subdirs; do
              mkdir -p "$hicolor_base/$size/$sub"
              if [ "$sub" = "stock" ]; then
                for stock_sub in $stock_subdirs; do
                  mkdir -p "$hicolor_base/$size/stock/$stock_sub"
                done
              fi
            done
          done

          mkdir -p "$hicolor_base/symbolic/apps"
          cp ${pkgs.hicolor-icon-theme}/share/icons/hicolor/index.theme $out/usr/share/icons/hicolor
      ''}
    ''}

    # Copy application packages
    ${lib.optionalString (apps != null) ''
      cp -r ${apps.package}/* $out/test/
    ''}

    # Copy benchmark packages
    ${lib.optionalString (benchmark != null) ''
      cp -r "${benchmark.package}"/* $out/benchmark/
    ''}

    # Copy syscall test packages
    ${lib.optionalString (syscall != null) ''
      cp -r "${syscall.package}"/opt/* $out/opt/

      # FIXME: Build gvisor syscall test with nix to avoid manual library copying.
      if [ "${syscall.testSuite}" == "gvisor" ]; then
        mkdir -p $out/lib/x86_64-linux-gnu
        cp -L ${gvisor_libs}/ld-linux-x86-64.so.2 $out/lib64/ld-linux-x86-64.so.2
        cp -L ${gvisor_libs}/libstdc++.so.6 $out/lib/x86_64-linux-gnu/libstdc++.so.6
        cp -L ${gvisor_libs}/libgcc_s.so.1 $out/lib/x86_64-linux-gnu/libgcc_s.so.1
        cp -L ${gvisor_libs}/libc.so.6 $out/lib/x86_64-linux-gnu/libc.so.6
        cp -L ${gvisor_libs}/libm.so.6 $out/lib/x86_64-linux-gnu/libm.so.6
      fi
    ''}

    # Copy Nix store dependencies
    # Use `writeClosure` to retrieve all dependencies of the specified packages.
    # This will generate a text file containing the complete closure of the packages,
    # including the packages themselves.
    # The output of `writeClosure` is equivalent to `nix-store -q --requisites`.
    mkdir -p $out/nix/store
    pkg_path=${lib.strings.concatStringsSep ":" all_pkgs}
    while IFS= read -r dep_path; do
      cp -a --no-preserve=ownership $dep_path $out/nix/store/
    done < ${writeClosure all_pkgs}
  '';
}