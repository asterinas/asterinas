{ lib
, stdenv
, fetchFromGitHub
}:

stdenv.mkDerivation rec {
  pname = "membench";
  version = "1.0.0";

  nativeBuildInputs = [];

  src = fetchFromGitHub {
    owner = "nicktehrany";
    repo = pname;
    rev = "master";
    sha256 = "sha256-5NLgwWbWViNBL1mQTXqoTnpwCNIC0lXoIeslWWnuXcE=";
  };

  enableParallelBuilding = true;

  installPhase = ''
    mkdir -p $out/bin
    cp membench $out/bin
  '';

  meta = {
    description = "Benchmarking Memory and File System Performance";
    mainProgram = "membench";
    longDescription = ''
      Benchmark to evaluate memory bandwidth/latency, page fault latency, and
      latency for mmap calls. Created for my BSc thesis on "Evaluating
      Performance Characteristics of the PMDK Persistent Memory Software Stack".
    '';
    homepage = "https://github.com/nicktehrany/membench";
    license = lib.licenses.mit;
    platforms = lib.platforms.unix;
  };
}
