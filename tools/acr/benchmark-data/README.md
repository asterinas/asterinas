# Benchmark Control Plane

[`src/benchmark/runner.py`](../src/benchmark/runner.py) uses detached Git worktrees and overlays only the reviewer package;
benchmark answer keys stay outside the reviewer checkout. The `fake` backend
exercises the control plane without a model; `openai-agents` runs a real review.
The YAML loader is installed with ACR's regular Python dependencies.

Run from the root of your Asterinas checkout:

```sh
tools/acr/benchmark-data/run.sh --problem 0002 --backend fake
```

With `--grade`, each grader result is validated to cover every numbered
expected defect exactly once. The runner prints strict recall
(`caught / expected`), reports `partial` separately, and names every partial or
missed defect with its `MATCH IF` criterion and grader reason. It also prints an
overall recall when several problems are selected.

See the organized [benchmark design](../spec/benchmark.md) for the schema,
isolation boundary, grading contract, and command options.
