# SPDX-License-Identifier: MPL-2.0

{ pkgs, ... }:

{
  environment.etc."dm-management-cases.sh".source = ./cases.sh;

  # The test configuration merger replaces this list rather than appending it.
  environment.systemPackages = with pkgs; [
    coreutils
    gawk
    gitMinimal
    gnugrep
    hello-asterinas
    lvm2
  ];
}
