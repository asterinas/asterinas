final: prev: {
  hello-asterinas = prev.stdenv.mkDerivation {
    name = "hello-asterinas";
    version = "0.1.0";
    buildCommand = ''
      mkdir -p $out/bin
      cat > $out/bin/hello-asterinas << 'EOF'
      #!/bin/sh
      echo "Hello Asterinas!"
      EOF
      chmod +x $out/bin/hello-asterinas
    '';
  };
}
