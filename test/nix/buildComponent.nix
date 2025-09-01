# Generic Nix builder for packages with optional patching, phase overrides, and meta.
# Arguments:
#   - pname, version, src: Basic package info
#   - patches: List of patch files
#   - argNativeBuildInputs, extraBuildInputs: Build inputs
#   - configureFlags: Flags for configure script
#   - argConfigurePhase, argPreConfigure, argBuildPhase, argInstallPhase: Shell overrides
#   - metaArgs: Override/add metadata for package

{ stdenv
, lib
, pkg-config
, python310 ? null
, automake ? null
, autoconf ? null
, perl ? null
, libtool ? null
}:

{ pname
, version
, src
, patches ? []
, argNativeBuildInputs ? []
, extraBuildInputs ? []
, prePatch ? []
, configureFlags ? []
, argConfigurePhase ? null
, argPreConfigure ? null
, argBuildPhase ? null
, argInstallPhase ? null
, metaArgs ? {}
}:
stdenv.mkDerivation rec {
  inherit pname version src patches;

  nativeBuildInputs = argNativeBuildInputs ++ lib.optional (pkg-config != null) pkg-config
    ++ lib.optional (python310 != null) python310
    ++ lib.optional (automake != null) automake
    ++ lib.optional (autoconf != null) autoconf
    ++ lib.optional (libtool != null) libtool
    ++ lib.optional (perl != null) perl;

  buildInputs = extraBuildInputs;

  patchPhase = ''
    ${lib.concatStringsSep "\n" prePatch}
    patchShebangs .
    if [ ! -z "${lib.concatStringsSep " " patches}" ]; then
      for patch in ${lib.concatStringsSep " " patches}; do
        patch -p1 < $patch
      done
    fi
  '';

  configurePhase = if argConfigurePhase != null then argConfigurePhase else ''
    echo "Running configurePhase for ${pname}"
    ./configure --prefix=$out ${lib.concatStringsSep " " configureFlags}
  '';

  preConfigure = if argPreConfigure != null then argPreConfigure else ''
    echo "Running configurePhase for ${pname}"
  '';

  buildPhase = if argBuildPhase != null then argBuildPhase else ''
    make -j$NIX_BUILD_CORES
  '';

  installPhase = if argInstallPhase != null then argInstallPhase else ''
    make install
  '';

  meta = {
    description = "Component: ${pname}";
    homepage = metaArgs.homepage or "https://www.x.org";
    license = metaArgs.license or lib.licenses.mit;
    platforms = metaArgs.platforms or lib.platforms.unix;
  } // metaArgs;
}