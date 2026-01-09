{ fetchurl, stdenv }:

let
  DCAP_VERSION = "DCAP_1.23";
  DCAP_URL_PREFIX =
    "https://github.com/intel/SGXDataCenterAttestationPrimitives/raw/${DCAP_VERSION}/QuoteGeneration/quote_wrapper";

  files = {
    tdx_attest_c = fetchurl {
      url = "${DCAP_URL_PREFIX}/tdx_attest/tdx_attest.c";
      sha256 = "08aijjx7jnmswimv4dhfwgbb0inwl0xg9hry37zy8k4wln6dys27";
    };
    tdx_attest_h = fetchurl {
      url = "${DCAP_URL_PREFIX}/tdx_attest/tdx_attest.h";
      sha256 = "0zsljf3gm9x0rp6dyin039akaf6lwf9fj0d6dskjzmlnsfzhqhmb";
    };
    test_tdx_attest_c = fetchurl {
      url = "${DCAP_URL_PREFIX}/tdx_attest/test_tdx_attest.c";
      sha256 = "1l7gx7wd2462ghwvf3i17kp7phq0sgyb22rpx568zlha48jqp9sc";
    };
    qgs_msg_lib_cpp = fetchurl {
      url = "${DCAP_URL_PREFIX}/qgs_msg_lib/qgs_msg_lib.cpp";
      sha256 = "0ffnmy8vg5yn12d9mz1zjdlfg98i9k112kyybr1fnm5yh1rdcnys";
    };
    qgs_msg_lib_h = fetchurl {
      url = "${DCAP_URL_PREFIX}/qgs_msg_lib/inc/qgs_msg_lib.h";
      sha256 = "092dvr5qbrwk707s0jwgqz79cw0dimp1n2qqkl9v6dik8l9fgfa6";
    };
  };
in stdenv.mkDerivation {
  pname = "dcap-quote-generation";
  version = DCAP_VERSION;

  dontUnpack = true;

  installPhase = ''
    mkdir -p $out/QuoteGeneration
    cp ${files.tdx_attest_c} $out/QuoteGeneration/tdx_attest.c
    cp ${files.tdx_attest_h} $out/QuoteGeneration/tdx_attest.h
    cp ${files.test_tdx_attest_c} $out/QuoteGeneration/test_tdx_attest.c
    cp ${files.qgs_msg_lib_cpp} $out/QuoteGeneration/qgs_msg_lib.cpp
    cp ${files.qgs_msg_lib_h} $out/QuoteGeneration/qgs_msg_lib.h
  '';
}
