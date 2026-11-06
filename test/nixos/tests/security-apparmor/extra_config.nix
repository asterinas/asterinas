{ lib, pkgs, ... }:

let
  upstream = "${pkgs.libapparmor.src}/parser/tst";
in
{
  environment.systemPackages = [
    pkgs.apparmor-parser
    pkgs.perl
  ];

  environment.etc."apparmor-parser-tests".source =
    pkgs.runCommand "apparmor-parser-tests-5.0.2" { }
      ''
        mkdir -p "$out/simple_tests" "$out/features_files"
        cp ${pkgs.libapparmor.src}/LICENSE "$out/LICENSE"
        cp ${upstream}/simple.pl ${upstream}/parser.conf ${upstream}/dirtest.sh "$out/"
        cp ${upstream}/features_files/features.all "$out/features_files/"
        cp -a ${upstream}/dirtest "$out/dirtest"
        for profile in simple_ok_no_rules.sd simple_named_ok_no_rules.sd profile_bad1.sd \
          simple_bad_no_program_name.sd simple_bad_no_opening_brace1.sd simple_bad_no_close_brace1.sd; do
          cp "${upstream}/simple_tests/profile/$profile" "$out/simple_tests/"
        done
      '';

  system.systemBuilderCommands = lib.mkAfter ''
    sed -i 's_ -- root=_ lsm=apparmor,yama -- root=_' $out/kernel-params
  '';
}
