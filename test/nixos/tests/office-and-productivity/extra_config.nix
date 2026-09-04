{ pkgs, ... }:

{
  environment.systemPackages = with pkgs; [
    mupdf
    pandoc
  ];
}
