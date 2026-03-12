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
  grafana_ini = pkgs.writeTextFile {
    name = "grafana.ini";
    text = ''
      [server]
      http_addr = 10.0.2.15
      http_port = 3000
      [paths]
      data = /tmp/grafana/data
      logs = /tmp/grafana/logs
      plugins = /tmp/grafana/plugins
      [plugins]
      preinstall_async = false
    '';
  };
in {
  environment.systemPackages = with pkgs; [ prometheus grafana ];

  environment.loginShellInit = ''
    [ ! -e /tmp/prometheus.yml ] && ln -s ${prometheus_yml} /tmp/prometheus.yml
    [ ! -e /tmp/grafana.ini ] && ln -s ${grafana_ini} /tmp/grafana.ini
  '';
}
