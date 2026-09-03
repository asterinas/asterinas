{ pkgs, ... }:
let
  test_pytorch = pkgs.writeTextFile {
    name = "test_pytorch.py";
    text = builtins.readFile ../../../../book/src/distro/popular-applications/ai-and-machine-learning/test_pytorch.py;
  };
  test_tensorflow = pkgs.writeTextFile {
    name = "test_tensorflow.py";
    text = builtins.readFile ../../../../book/src/distro/popular-applications/ai-and-machine-learning/test_tensorflow.py;
  };
in
{
  environment.systemPackages = with pkgs; [
    (python3.withPackages (
      p: with p; [
        torch
        tensorflow
        pytest
      ]
    ))
    codex
    ollama
  ];

  system.activationScripts.testFixtures = ''
    ln -sfT ${test_pytorch} /tmp/test_pytorch.py
    ln -sfT ${test_tensorflow} /tmp/test_tensorflow.py
  '';
}
