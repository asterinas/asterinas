self: super:

{
  nixos-rebuild-ng = super.nixos-rebuild-ng.overrideAttrs (oldAttrs: {
    patches = (oldAttrs.patches or [ ]) ++ [ ./0001-Bypass-system-dbus.patch ];
  });
}
