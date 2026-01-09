{ fetchurl, stdenv }:
let
  prefix = "https://raw.githubusercontent.com/cesanta/mongoose/refs/tags/7.13";
  files = {
    mongoose_c = fetchurl {
      url = "${prefix}/mongoose.c";
      sha256 = "sha256-HEIVSh09Ia2UEuNg59CJxH5vBerlmDeUyYHqXgWVapA";
    };
    mongoose_h = fetchurl {
      url = "${prefix}/mongoose.h";
      sha256 = "sha256-JMqSTzO5c9qlptzN8mE84eB9DWiq6Fb3DlLVyPcQ498";
    };
  };
in stdenv.mkDerivation {
  pname = "mongoose";
  version = "0.1.0";
  buildCommand = ''
    mkdir -p $out
    cp ${files.mongoose_c} $out/mongoose.c
    cp ${files.mongoose_h} $out/mongoose.h
  '';
}
