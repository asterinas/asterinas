{ pkgs, ... }:

{
  environment.systemPackages = with pkgs; [
    ffmpeg
    sox
    imagemagick
  ];
}
