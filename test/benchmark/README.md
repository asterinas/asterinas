# Introduction to benchmarks

## Overview of supported benchmarks
The benchmark suite contains several benchmarks that can be used to evaluate the performance of the Asterinas platform. The following benchmarks are supported:

- [Sysbench](#Sysbench)
- [Membench](#Membench)
- [Iperf](#Iperf)

### Sysbench
Sysbench is a scriptable benchmark tool that evaluates system performance. It includes five kinds of tests: CPU, memory, file I/O, mutex performance, and thread performance. Detailed usage and options can be found by using:
```shell
sysbench --help
sysbench --test=<test_name> help
``` 
Here we list some general commands for evaluation:
```shell
# CPU test
sysbench --test=cpu --cpu-max-prime=<N> --num-threads=<N> run

# Thread test
sysbench --test=threads --thread-yields=<N> --num-threads=<N> --max-time=<N> run

# Mutex test
sysbench --test=mutex --mutex-num=<N> --mutex-locks=<N> --num-threads=<N>

# File test, the file-total-size and file-num of prepare and run must be consistent 
sysbench --test=fileio --file-total-size=<N><K,M,G> --file-num=<N> prepare
sysbench --test=fileio --file-total-size=<N><K,M,G> --file-num=<N> --file-test-mode=<Type> --file-block-size=<N><K,M,G> --max-time=<N> run

# Memory test
sysbench --test=memory --memory-block-size=<N><K,M,G> --memory-access-mode=<Type> --memory-oper=<Type> run
```

### Membench
Membench is used to establish a baseline for memory bandwidth and latency. For specific usage and options, use:
```shell
membench --help
``` 
Here we list some general commands to use membench:
```shell
# Measure the latency of mmap
membench -runtime=5 -dir=/dev/zero -size=<N><K,M,G> -engine=mmap_lat

# Measure the latency of page fault handling. The size must be consistent with the file size.
membench -runtime=5 -dir=path_to_a_file -size=<N><K,M,G> -copysize=<N><K,M,G> -mode=<Type> -engine=page_fault 

# This is a easy way to generate a file with target size in Linux.
# The following command can create a file named 512K.file with the size 512K.
dd if=/dev/zero of=512K.file bs=1K count=512
```

### Iperf
iPerf is a tool for actively measuring the maximum achievable bandwidth on IP networks. Usage and options are detailed in:
```shell
iperf3 -h
``` 
iperf can run in the following instructions:
```shell
export HOST_ADDR=127.0.0.1
export HOST_PORT=8888
iperf3 -s -B $HOST_ADDR -p $HOST_PORT -D # Start the server as a daemon
iperf3 -c $HOST_ADDR -p $HOST_PORT # Start the client
```
Note that [a variant of iperf3](https://github.com/stefano-garzarella/iperf-vsock) can measure the performance of `vsock`. But the implemented `vsock` has not been verified to work well in it.

## Add benchmark to benchmark CI

To add a new benchmark to the Asternias Continuous Integration (CI) system, follow these detailed steps:

### Step 1: Add the Benchmark to the `asterinas/test/benchmarks` Directory

1. **Create the Benchmark Directory:**
   - Navigate to `asterinas/test/benchmarks`.
   - Create a new directory named after your benchmark, e.g., `lmbench/getpid`.

2. **Create the Necessary Files:**
   - **config.json:**
     ```json
      {
        "alert_threshold": "125%",
        "alert_tool": "customSmallerIsBetter",
        "search_pattern": "Simple syscall:",
        "result_index": "3",
        "description": "lat_syscall null",
        "title": "[Process] The cost of getpid",
        "benchmark_type": "host_guest"
      } 
     ```
     
    - `alert_threshold`: Set the threshold for alerting. If the benchmark result exceeds this threshold, an alert will be triggered. Note that the threshold should usually be greater than 100%. If your results are not stable, set it to a bigger value.
    - `alert_tool`: Choose the validation tool to use. The available options are `customBiggerIsBetter` and `customSmallerIsBetter`. Refer to [this](https://github.com/benchmark-action/github-action-benchmark?tab=readme-ov-file#tool-required) for more details. If using `customBiggerIsBetter`, the alert will be triggered when `prev.value / current.value` exceeds the threshold. If using `customSmallerIsBetter`, the alert will be triggered when `current.value / prev.value` exceeds the threshold.
    - `search_pattern`: Define a regular expression to extract benchmark results from the output using `awk`. This regular expression is designed to match specific patterns in the output, effectively isolating the benchmark results and producing a set of fragments.
    - `result_index`: Specify the index of the result in the extracted output. This field is aligned with `awk`'s action.
    - `description`: Provide a brief description of the benchmark.
    - `title`: Set the title of the benchmark.
    - `benchmark_type`: This parameter defines the type of benchmark to be executed. The default value is `guest_only`. The available options include `guest_only`, and `host_guest`.
      - `guest_only`: Use this option when the benchmark is intended solely for the guest environment.
      - `host_guest`: Choose this option when the benchmark involves both the host and guest environments. When using this option, you will need to define your own `host.sh` and `bench_runner.sh` scripts to handle the host-side operations and benchmark execution.

    For example, if the benchmark output is "Syscall average latency: 1000 ns", the `search_pattern` is "Syscall average latency:", and the `result_index` is "4". `awk` will extract "1000" as the benchmark result. See the `awk` [manual](https://www.gnu.org/software/gawk/manual/gawk.html#Getting-Started) for more information.

    - **summary.json:**
    ```json
    {
        "benchmarks": [
            "cpu_lat",
            "thread_lat"
        ]
    }
    ```
    - List all the benchmarks that are included in the benchmark overview. This file is used to generate the overview chart of the benchmark results. 
    - The benchmark does not appear in the overview chart if it is not listed in this file. But it will still be included in the detailed benchmark results.
    - The sequence of the benchmarks in this file will be the same as the sequence in the overview chart.

   - **result_template.json:**
     ```json
     [
         {
             "name": "Average Syscall Latency on Linux",
             "unit": "ns",
             "value": 0,
             "extra": "linux_result"
         },
         {
             "name": "Average Syscall Latency on Asterinas",
             "unit": "ns",
             "value": 0,
             "extra": "aster_result"
         }
     ]
     ```
     - Adjust `name` and `unit` according to your benchmark specifics.

   - **run.sh:**
     ```bash
     #!/bin/bash

     /benchmark/bin/lmbench/lat_syscall -P 1 null
     ```
     - This script runs the benchmark. Ensure the path to the benchmark binary is correct. `asterinas/test/Makefile` handles the benchmark binaries.

### Step 2: Update the `asterinas/.github/benchmarks.yml` File

1. **Edit the Benchmarks Configuration:**
   - Open `asterinas/.github/benchmarks.yml`.
   - Add your benchmark to the `strategy.matrix.benchmark` list:
     ```yaml
     strategy:
       matrix:
         benchmark: [lmbench/getpid]
       fail-fast: false
     ```

### Step 3: Test the Benchmark Locally

1. **Run the Benchmark:**
   - Execute the following command to test the benchmark locally:
     ```bash
     cd asterinas
     bash test/benchmark/bench_linux_and_aster.sh lmbench/getpid
     ```
   - Ensure the benchmark runs successfully and check the results in `asterinas/result_getpid.json`.

### Additional Considerations

- **Validation:** After adding and testing the benchmark, ensure that the CI pipeline correctly integrates the new benchmark by triggering a CI build.
- **Documentation:** Update any relevant documentation to include the new benchmark, explaining its purpose and how to interpret its results.

By following these steps, you will successfully integrate a new benchmark into the Asternias CI system, enhancing its capability to evaluate platform performance.
