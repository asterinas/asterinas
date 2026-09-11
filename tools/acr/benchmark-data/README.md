# Benchmark Data

[`problems.yaml`](problems.yaml) holds the known-defect problems.
Run them from the root of an Asterinas checkout with [`run_benchmark.sh`](../run_benchmark.sh):

```sh
tools/acr/run_benchmark.sh --problem 0002 --config tools/acr/tests/acr.fake.toml
```

See [Benchmark and evaluation](../spec/benchmark.md)
for the problem schema, the isolation boundary, grading, and every command option.
