# SPDX-License-Identifier: MPL-2.0

{ pkgs, ... }:
{
  environment.systemPackages = with pkgs; [
    lvm2
  ];
}
