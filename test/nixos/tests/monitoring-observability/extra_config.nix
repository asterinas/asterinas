{ config, lib, pkgs, ... }:
let
  prometheus_yml = pkgs.writeTextFile {
    name = "prometheus.yml";
    text = ''
      global:
        scrape_interval: 15s
      scrape_configs:
        - job_name: "prometheus"
          static_configs:
            - targets: [ "10.0.2.15:9090" ]
    '';
  };
in {
  environment.systemPackages = with pkgs; [ prometheus grafana ];

  environment.loginShellInit = ''
    [ ! -e /tmp/prometheus.yml ] && ln -s ${prometheus_yml} /tmp/prometheus.yml
  '';
}
