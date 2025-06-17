{ pkgs ? import <nixpkgs> { }, syscallTestWorkDir ? "/tmp", }:
let ltp = pkgs.callPackage ./ltp.nix { inherit pkgs; };
in pkgs.stdenv.mkDerivation {
  pname = "syscall_test";
  version = "0.1.0";
  src = pkgs.lib.fileset.toSource {
    root = ./..;
    fileset = ../syscall_test;
  };

  buildCommand = ''
    ltp_dir=$src/syscall_test/ltp
    target_dir=$out/opt/ltp
    mkdir -p $target_dir/testcases/bin
    mkdir -p $target_dir/runtest

    awk '!/^#/ && NF' $ltp_dir/testcases/all.txt > $target_dir/all.txt

    if [ ${syscallTestWorkDir} = "/ext2" ]; then
      grep -vxF -f $ltp_dir/testcases/blocked/ext2.txt $target_dir/all.txt > $target_dir/filtered.txt
    elif [ ${syscallTestWorkDir} = "/exfat" ]; then
      grep -vxF -f $ltp_dir/testcases/blocked/exfat.txt $target_dir/all.txt > $target_dir/filtered.txt
    else
      cp -f $target_dir/all.txt $target_dir/filtered.txt
    fi

    while IFS= read -r testcase; do
      matching_line=$(grep -E "^$testcase\s" ${ltp}/runtest/syscalls)
      if [ -z "$matching_line" ]; then
        echo "Warning: Test case $testcase not found in ${ltp}/runtest/syscalls" >&2
        continue
      fi

      bin_file=$(echo "$matching_line" | awk '{print $2}')
      if [ -z "$bin_file" ]; then
        echo "Warning: Parsing bin file for $case failed" >&2
        continue
      fi

      bin_path="${ltp}/testcases/bin/$bin_file"
      if [ -f "$bin_path" ]; then
        cp -u "$bin_path" $target_dir/testcases/bin/
      else
        echo "Warning: Test case $testcase binary not found in ${ltp}/testcases/bin" >&2
      fi

      echo "$matching_line" >> $target_dir/runtest/syscalls
    done < $target_dir/filtered.txt

    rm -f $target_dir/all.txt $target_dir/filtered.txt

    cp -r ${ltp}/bin $target_dir/
    cp -r ${ltp}/runltp $target_dir/
    cp -r ${ltp}/Version $target_dir/
    cp -r ${ltp}/ver_linux $target_dir/
    cp -r ${ltp}/IDcheck.sh $target_dir/
    cp -r $ltp_dir/run_ltp_test.sh $target_dir/

    mkdir -p $out/opt/syscall_test
    cp -r $src/syscall_test/run_syscall_test.sh $out/opt/syscall_test/
  '';
}
