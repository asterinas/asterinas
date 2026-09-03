{ pkgs, ... }:

{
  environment.systemPackages = with pkgs; [
    just
    go-task
    goreleaser
    git
    go
  ];
}
