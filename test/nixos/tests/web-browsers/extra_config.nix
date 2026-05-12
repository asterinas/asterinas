{ pkgs, ... }:

{
  environment.systemPackages = with pkgs; [ links2 w3m ];
}
