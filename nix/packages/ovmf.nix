# SPDX-License-Identifier: MPL-2.0
#
# OVMF firmware in the layout expected by tools/qemu_args.sh:
# $out/{OVMF.fd,OVMF_VARS.fd} and $out/microvm/MICROVM.fd.
#
# The caller passes pkgsCross.gnu64.{edk2,OVMF}; the overlay pins edk2 before
# that cross package set is evaluated, so both firmware builds use the Docker
# image's edk2 tag.
{ edk2, OVMF, runCommand, nasm, acpica-tools }:

let
  # The standard OVMF package does not include MICROVM.fd, so build the
  # MicrovmX64 platform separately.
  #
  # The bare edk2 helper does not add the assembler or ACPI compiler.
  microvm = edk2.mkDerivation "OvmfPkg/Microvm/MicrovmX64.dsc" {
    name = "ovmf-microvm-x64";
    nativeBuildInputs = [ nasm acpica-tools ];
    # Match nixpkgs' OVMF hardening profile for freestanding firmware.
    hardeningDisable = [ "format" "stackprotector" "pic" "fortify" ];
  };
in runCommand "asterinas-ovmf" { } ''
  mkdir -p $out
  cp ${OVMF.fd}/FV/OVMF.fd      $out/OVMF.fd
  cp ${OVMF.fd}/FV/OVMF_VARS.fd $out/OVMF_VARS.fd
  mkdir -p $out/microvm
  cp ${microvm}/FV/MICROVM.fd $out/microvm/MICROVM.fd
''
