# Monitoring & Observability

This category covers metrics, logging, and tracing tools.

## Metrics & Alerting

### Prometheus

[Prometheus](https://prometheus.io/) is a monitoring and alerting toolkit with a time-series database.

#### Installation

```nix
environment.systemPackages = [ pkgs.prometheus ];
```

#### Verified Usage

```bash
# Run Prometheus with a specific config
prometheus --config.file=/tmp/prometheus.yml --web.listen-address="10.0.2.15:9090"
```
