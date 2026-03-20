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

### TODO: Grafana

[Grafana](https://grafana.com/) is an open-source platform for data visualization and monitoring dashboards.

## Logging

### TODO: Fluentd

[Fluentd](https://www.fluentd.org/) is an open-source data collector for unified logging.

### TODO: Grafana Loki

[Loki](https://grafana.com/oss/loki/) is a log aggregation system designed to complement Grafana.

## Tracing

### TODO: Jaeger

[Jaeger](https://www.jaegertracing.io/) is a distributed tracing platform.

### TODO: OpenTelemetry Collector

[OpenTelemetry Collector](https://opentelemetry.io/docs/collector/) is a vendor-agnostic implementation for receiving, processing, and exporting telemetry data.
