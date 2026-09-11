# Benchmark Control Plane

`runner.py` uses detached Git worktrees and overlays only the reviewer package;
benchmark answer keys stay outside the reviewer checkout. The `fake` backend
exercises the control plane without a model; `openai-agents` runs a real review.
Install the optional `acr[benchmark]` dependency for YAML problem loading.

```sh
/path/to/acr/benchmark/run.sh --problem 0002 --backend fake
```

With `--grade`, each grader result is validated to cover every numbered
expected defect exactly once. The runner prints strict recall
(`caught / expected`), reports `partial` separately, and names every partial or
missed defect with its `MATCH IF` criterion and grader reason. It also prints an
overall recall when several problems are selected.

See the organized [benchmark design](../spec/benchmark.md) for the schema,
isolation boundary, grading contract, and command options.
