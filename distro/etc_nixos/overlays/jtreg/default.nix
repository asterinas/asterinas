final: prev: {
  jtreg = let
    # JT Harness
    jtharness = prev.stdenv.mkDerivation rec {
      pname = "jtharness";
      version = "6.0-b24";
      src = prev.fetchzip {
        url =
          "https://github.com/openjdk/jtharness/archive/refs/tags/jt${version}.zip";
        sha256 = "sha256-41PjFHBrtcNN/PgUmZQloE0oXBWEv9l6YqPIdVgpymo";
      };
      buildInputs = with prev.pkgs; [ ant openjdk21 ];
      buildCommand = ''
        mkdir -p $out

        BUILD_DIR=$(mktemp -d)
        ant -DBUILD_DIR=$BUILD_DIR -f $src/build/build.xml dist
        cp -r $BUILD_DIR/binaries/* $out
      '';
    };

    # AsmTools
    asmtools = prev.stdenv.mkDerivation rec {
      pname = "asmtools";
      version = "7.0-b09";
      src = prev.fetchzip {
        url =
          "https://github.com/openjdk/asmtools/archive/refs/tags/${version}.zip";
        sha256 = "sha256-fRlXq+c09MyMfVRoIEdx6egusWVDPiYRDjC3rPmRZTY";
      };
      buildInputs = with prev.pkgs; [ ant openjdk21 ];
      buildCommand = ''
        mkdir -p $out

        BUILD_DIR=$(mktemp -d)
        ant -DBUILD_DIR=$BUILD_DIR -f $src/build/build.xml release
        cp -r $BUILD_DIR/release/* $out
      '';
    };

    # JUnit Platform Console Standalone (includes JUnit Jupiter, JUnit Vintage, and dependencies)
    junit = prev.stdenv.mkDerivation rec {
      pname = "junit";
      version = "1.8.2";

      junit = prev.fetchurl {
        url =
          "https://repo1.maven.org/maven2/org/junit/platform/junit-platform-console-standalone/${version}/junit-platform-console-standalone-${version}.jar";
        sha256 = "sha256-3EmPI0Io+ByBi+z7a39x9z3ysOmV+T8lwF/O0Md+Y1Y";
      };

      license = prev.fetchurl {
        url =
          "https://github.com/junit-team/junit-framework/raw/refs/heads/main/LICENSE.md";
        sha256 = "sha256-WqTNRMERrdF40cLi/jbVikhAEsgBZ9+SX4Js1k1BG/A";
      };

      buildCommand = ''
        mkdir -p $out/lib

        cp ${license} $out/LICENSE.md
        cp ${junit} $out/lib/junit-platform-console-standalone.jar
      '';
    };

    # TestNG and its dependencies
    testng = prev.stdenv.mkDerivation rec {
      pname = "testng";
      version = "7.3.0";

      testng = prev.fetchurl {
        url =
          "https://repo1.maven.org/maven2/org/testng/testng/${version}/testng-${version}.jar";
        sha256 = "sha256-Y3J0iPlxfVfw0KD+5aH8EKK+nPz/LsOnGHZW1mPAd04";
      };

      license = prev.fetchurl {
        url =
          "https://github.com/testng-team/testng/raw/refs/tags/${version}/LICENSE.txt";
        sha256 = "sha256-wbnfEnXnafPbqwANHkV6LUsPKOtdpsd+SNw37rogLtc";
      };

      jcommander = prev.fetchurl {
        url =
          "https://repo1.maven.org/maven2/com/beust/jcommander/1.78/jcommander-1.78.jar";
        sha256 = "sha256-eJHeu4S1+D6b1XWT6+zjOZq74P2TjPMGs1NMV5E7lhU";
      };

      guice = prev.fetchurl {
        url =
          "https://repo1.maven.org/maven2/com/google/inject/guice/4.2.3/guice-4.2.3.jar";
        sha256 = "sha256-oh5Q/7tn563FtGz3ueGkgPHg8E/UIB3bHGXakSkGAa8";
      };

      buildCommand = ''
        mkdir -p $out/lib

        cp ${license} $out/LICENSE.txt
        cp ${testng} $out/lib/testng.jar
        cp ${jcommander} $out/lib/jcommander.jar
        cp ${guice} $out/lib/guice.jar
      '';
    };
  in prev.stdenv.mkDerivation rec {
    pname = "jtreg";
    version = "7.1.1";
    number = "1";
    src = prev.fetchzip {
      url =
        "https://github.com/openjdk/jtreg/archive/refs/tags/jtreg-${version}+${number}.zip";
      sha256 = "sha256-O8z1gJIKaAQq0BxBRkGki/82CpBy33gkBeX3e3wKXGE";
    };

    patches = [ ./0001-Fix-tool-paths-and-build-flags.patch ];

    JAVATEST_JAR = "${jtharness}/lib/javatest.jar";
    JTHARNESS_NOTICES = "${jtharness}/legal/copyright.txt ${jtharness}/LICENSE";

    ASMTOOLS_JAR = "${asmtools}/lib/asmtools.jar";
    ASMTOOLS_NOTICES = "${asmtools}/LICENSE";

    JUNIT_JARS = "${junit}/lib/junit-platform-console-standalone.jar";
    JUNIT_NOTICES = "${junit}/LICENSE.md";

    TESTNG_JARS =
      "${testng}/lib/testng.jar ${testng}/lib/jcommander.jar ${testng}/lib/guice.jar";
    TESTNG_NOTICES = "${testng}/LICENSE.txt";

    JDKHOME = "${prev.pkgs.openjdk21}";
    JAVA_SPECIFICATION_VERSION = "21";

    buildInputs = with prev.pkgs; [
      ant
      openjdk21
      hostname
      pandoc
      perl
      html-tidy
      unzip
      zip
    ];
    buildPhase = ''
      make BUILD_VERSION=${version} BUILD_NUMBER=${number} -C make
    '';
    installPhase = ''
      mkdir $out
      cp -r build/images/jtreg/* $out
    '';
  };
}
