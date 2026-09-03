{
  config,
  lib,
  pkgs,
  ...
}:

/*
  Module: XFCE desktop defaults (wallpaper and icons)
  Purpose:
    - Provide system-wide default XFCE desktop settings via /etc/xdg/xfce4/xfconf/xfce-perchannel-xml/xfce4-desktop.xml.
    - Set a default wallpaper and desktop icon visibility (home, filesystem, removable, trash).

  How it integrates with XFCE:
    - XFCE reads settings from the xfconf daemon (channel: "xfce4-desktop").
    - If a per-user file exists at ~/.config/xfce4/xfconf/xfce-perchannel-xml/xfce4-desktop.xml,
      that overrides the system default in /etc/xdg. This module does NOT override per-user settings.
*/

let
  wallpaper = pkgs.fetchurl {
    url = "https://raw.githubusercontent.com/asterinas/asterinas-artwork/f92b04a998f16c0b11f22987181a67c9106c3684/aster_nixos/v0.18.0/wallpaper_berry-madjidi_unsplash_1625x1080.png";
    sha256 = "0y6r8nq9gp05nlpk1s9fscs0jcj70pxhxaim698q9lfwfqkidlhz";
  };

  xfceDesktopXml = pkgs.writeText "xfce4-desktop.xml" ''
    <?xml version="1.0" encoding="UTF-8"?>
    <channel name="xfce4-desktop" version="1.0">
      <!--
        The 'last-settings-migration-version' property tracks the last migration version
        of the XFCE desktop settings. This should be incremented if the settings format
        changes and a migration is required. See XFCE documentation for details.
      -->
      <property name="last-settings-migration-version" type="uint" value="1"/>
      <property name="backdrop" type="empty">
        <property name="screen0" type="empty">
          <property name="monitor0" type="empty">
            <property name="workspace0" type="empty">
              <property name="last-image" type="string" value="${wallpaper}"/>
              <!-- image-style=5 means "zoomed" wallpaper mode in XFCE -->
              <property name="image-style" type="int" value="5"/>
            </property>
          </property>
          <property name="monitordefault" type="empty">
            <property name="workspace0" type="empty">
              <property name="last-image" type="string" value="${wallpaper}"/>
              <!-- image-style=5 means "zoomed" wallpaper mode in XFCE -->
              <property name="image-style" type="int" value="5"/>
            </property>
          </property>
        </property>
      </property>
      <property name="desktop-icons" type="empty">
        <property name="file-icons" type="empty">
          <property name="show-home" type="bool" value="true"/>
          <property name="show-filesystem" type="bool" value="true"/>
          <property name="show-removable" type="bool" value="true"/>
          <property name="show-trash" type="bool" value="true"/>
        </property>
        <property name="icon-size" type="uint" value="48"/>
      </property>
    </channel>
  '';

  xfceXMLPath = "xdg/xfce4/xfconf/xfce-perchannel-xml/xfce4-desktop.xml";
in
lib.mkIf (config.services.xserver.enable && config.services.xserver.desktopManager.xfce.enable) {
  environment.etc.${xfceXMLPath} = {
    source = xfceDesktopXml;
    mode = "0644";
  };
}
