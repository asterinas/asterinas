# Benchmark Data

[`problems.yaml`](problems.yaml) holds the known-defect problems,
and [`run.sh`](run.sh) runs them from the root of an Asterinas checkout:

```sh
tools/acr/benchmark-data/run.sh --problem 0002 --config tools/acr/tests/acr.fake.toml
```

See [Benchmark and evaluation](../spec/benchmark.md)
for the problem schema, the isolation boundary, grading, and every command option.
