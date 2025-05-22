{ pkgs ? import <nixpkgs> { }, }:
with pkgs;
stdenv.mkDerivation {
  pname = "membench";
  version = "0.1.0";
  src = fetchFromGitHub {
    owner = "nicktehrany";
    repo = "membench";
    rev = "91f4e5b142df05e501d8941b555d547ed4958152";
    sha256 = "sha256-5NLgwWbWViNBL1mQTXqoTnpwCNIC0lXoIeslWWnuXcE=";
  };
  enableParallelBuilding = true;
  makeFlags = [ "CC=${stdenv.cc.targetPrefix}cc" ];
  installPhase = ''
    mkdir -p $out
    cp membench $out/
  '';
}
