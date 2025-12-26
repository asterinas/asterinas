# This file is used to generate a final NixOS configuration by merging the
# base configuration with an extra configuration module.

{ pkgs ? import <nixpkgs> { }, configName, configsDir ? ./configs }:

let
  lib = pkgs.lib;

  # Finds the path to the extra configuration file based on the `configName`.
  # Throws an error if the file for the specified name doesn't exist.
  resolveConfig = name:
    let candidate = "${configsDir}/${name}.nix";
    in if builtins.pathExists candidate then
      candidate
    else
      throw "Configuration for name '${name}' not found at: ${candidate}";

  baseText = builtins.readFile ./configuration.nix;
  extraConfigText = builtins.readFile (resolveConfig configName);

  indent = text:
    lib.concatMapStrings (line: "    ${line}\n") (lib.splitString "\n" text);

  # The string content for the generated `configuration.nix`.
  finalConfigText = ''
        { config, lib, pkgs, ... }:
        let
          baseModule =
    ${indent baseText}
          ;
          extraModule =
    ${indent extraConfigText}
          ;

          base = baseModule { inherit config lib pkgs; };
          extra = extraModule { inherit config lib pkgs; };
        in
          # Deeply merge the base, extra, and common configurations.
          # The module system will combine all `environment.systemPackages` lists.
          lib.recursiveUpdate base extra
  '';
in pkgs.writeText "configuration.nix" finalConfigText
