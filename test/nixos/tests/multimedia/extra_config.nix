{ config, lib, pkgs, ... }:

{
  environment.systemPackages = with pkgs; [ file ffmpeg sox imagemagick ];
}
