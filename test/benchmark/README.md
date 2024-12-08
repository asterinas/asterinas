# Asterinas Benchmark Collection

The Asterinas Benchmark Collection evaluates the performance of Asterinas in comparison to Linux across a range of benchmarking tools (e.g., LMbench, Sysbench, iPerf) and real-world applications (e.g., Nginx, Redis, SQLite). These benchmarks are conducted under various configurations, such as within a single VM or between a VM and its host.

The benchmarks are run automatically on a nightly basis through CI pipelines. Results, presented in clear and visually appealing figures and tables, are available [here](https://asterinas.github.io/benchmark/).

## File Organization

### Benchmark Suites

The benchmark collection is organized into benchmark suites, each dedicated to a specific benchmarking tool or application. These suites focus on comparing the performance of different operating systems using a particular methodology. Currently, there are seven benchmark suites, each located in its own directory:

- [lmbench](https://github.com/asterinas/asterinas/tree/main/test/benchmark/lmbench)
- [sysbench](https://github.com/asterinas/asterinas/tree/main/test/benchmark/sysbench)
- [fio](https://github.com/asterinas/asterinas/tree/main/test/benchmark/fio)
- [iperf](https://github.com/asterinas/asterinas/tree/main/test/benchmark/iperf)
- [sqlite](https://github.com/asterinas/asterinas/tree/main/test/benchmark/sqlite)
- [redis](https://github.com/asterinas/asterinas/tree/main/test/benchmark/redis)
- [nginx](https://github.com/asterinas/asterinas/tree/main/test/benchmark/nginx)

Each suite has a corresponding web page (e.g., [LMbench results](https://asterinas.github.io/benchmark/lmbench/)) that publishes the latest performance data. At the top of each page, a summary table showcases the most recent results, configured using the `summary.json` file in the suite's directory.

### Benchmark Jobs

Each benchmark suite is divided into benchmark jobs, which perform specific benchmarking tasks. Benchmark jobs are organized into subdirectories under their parent suite directory:

```plaintext
<bench_suite>/
├── <bench_job_a>/
└── <bench_job_b>/
```

Benchmark jobs can be executed using the `bench_linux_and_aster.sh` script located in the `test/benchmark/` directory:

```bash
./bench_linux_and_aster.sh <bench_suite>/<bench_job>
```

For example, to measure the latency of the `getppid` system call on both Linux and Asterinas, run:

```bash
./bench_linux_and_aster.sh lmbench/process_getppid_lat
```

The script starts a VM running either Linux or Asterinas as the guest OS and invokes the `run.sh` script located in the benchmark job's directory to execute the benchmark:

```plaintext
<bench_suite>/
└── <guest_only_job>/
    └── run.sh
```

For benchmarks requiring collaboration between the guest VM and the host OS (e.g., server-client scenarios), the job should include a `host.sh` script alongside the `run.sh` script:

```plaintext
<bench_suite>/
└── <host_guest_job>/
    ├── host.sh
    └── run.sh
```

### Benchmark Results

Each benchmark job produces one or more benchmark results, which are saved in a structured format.

#### Single Result Jobs

For jobs that produce a single result, the directory is structured as follows:

```plaintext
<bench_suite>/
└── <single_result_job>/
    ├── bench_result.json
    └── run.sh
```

The `bench_result.json` file contains metadata about the result, including the title, measurement unit, and whether higher or lower values indicate better performance.

#### Multi-Result Jobs

For jobs producing multiple results, the directory includes a `bench_results/` folder:

```plaintext
<bench_suite>/
└── <multi_result_job>/
    ├── bench_results/
    │   ├── <job_a>.json
    │   └── <job_b>.json
    └── run.sh
```

Each JSON file in the `bench_results/` directory describes a specific result's metadata.

## Adding New Benchmarks

To add a new benchmark to the Asterinas Continuous Integration (CI) system, follow the steps below. These instructions are tailored to the directory structure outlined earlier, where benchmarks are organized under specific suites and jobs.

### Step 1: Add a New Benchmark Job

Each benchmark job should be added under the corresponding suite in the `test/benchmark` directory.

#### Directory Structure

```plaintext
<bench_suite>/
└── <job>/
    ├── host.sh # Only for host-guest jobs
    ├── bench_result.json  # or bench_results/ directory for multiple results jobs
    └── run.sh
```

### Step 2: Create Necessary Files

1. **`run.sh`**  
   This script executes the benchmark within the guest VM. Example for `iperf3` benchmark:

   ```bash
   #!/bin/bash
   echo "Running iperf3 server..."
   /benchmark/bin/iperf3 -s -B 10.0.2.15 --one-off
   ```

2. **`host.sh`** (only for host-guest jobs)  
   This script handles host-side operations.

   ```bash
   #!/bin/bash

   echo "Running iperf3 client"
   iperf3 -c $GUEST_SERVER_IP_ADDRESS -f m
   ```

3. **`bench_result.json`** (single-result job)  
   Contains metadata about the benchmark result:

   ```json
   {
    "alert_threshold": "130%",
    "alert_tool": "customBiggerIsBetter",
    "search_pattern": "sender",
    "result_index": "7",
    "description": "iperf3 -s -B 10.0.2.15",
    "title": "[Network] iperf3 sender performance using TCP",
    "unit": "Mbits/sec",
    "legend": "Average TCP Bandwidth over virtio-net between Host Linux and Guest {system}"
   }   
   ```
   See the [`bench_result.json` format](#the-bench_resultjson-format) section for more information.

4. **`bench_results/`** (multi-result job)  
   Each result file provides metadata for an individual metric:

   ```plaintext
   // ext2_deletes_between.json.json
   {
    "alert_threshold": "125%",
    "alert_tool": "customSmallerIsBetter",
    "search_pattern": "10000 DELETEs, numeric BETWEEN, indexed....",
    ...
   }
   ```

   ```plaintext
   // ext2_updates_between.json
   {
    "alert_threshold": "125%",
    "alert_tool": "customSmallerIsBetter",
    "search_pattern": "10000 UPDATES, numeric BETWEEN, indexed....",
    ...
   }
   ```

### Step 3: Update Suite's `summary.json`

Edit the `summary.json` file at the root of the suite. Taking `sqlite` for example:

```json
{
    "benchmarks": [
        "sqlite/ext2_deletes_between",
        "sqlite/ext2_deletes_individual",
        "sqlite/ext2_refill_replace",
        "sqlite/ext2_selects_ipk"
    ]
}
```

This file lists all benchmarks included in the summary table on the suite's web page.

### Step 4: Update the CI Configuration

1. Open `.github/benchmarks.yml`.
2. Add your new benchmark job to the matrix:

   ```yaml
   strategy:
     matrix:
       benchmarks:
         - redis/ping_inline_100k_conc20_rps
         - sqlite/ext2_benchmarks
         ...
   ```

### Step 5: Test Locally

Run the benchmark locally to ensure it works as expected:

```bash
cd asterinas/
bash test/benchmark/bench_linux_and_aster.sh <bench_suite>/<new_benchmark_job>
```

Check that the results are saved as `result_<bench_suite>-<new_benchmark_job>.json` under `asterinas/`.

### Step 6: Validate and Commit

- Modify the `runs-on` field from `self-hosted` to `ubuntu-latest` on `.github/benchmarks.yml` to trigger the CI pipeline on your own repository first.
- Ensure CI pipelines correctly execute the new benchmark.
- Reverse the `runs-on` field back to `self-hosted` after validation.
- Commit your changes and push them to the repository.

By following these steps, you can seamlessly integrate new benchmarks into the Asterinas Benchmark Collection.

## The `bench_result.json` Format

The `bench_result.json` file serves as a configuration for how the benchmark result will be processed and displayed. Below is the format:

```plaintext
{
    /* 
        Configuration for performance alerts.
    */
    "alert": {
        /* 
            Defines the threshold for performance alerts. 
            If the benchmark result deviates beyond this threshold compared to a baseline, an alert will be triggered.
            
            Format: Percentage string (e.g., "130%" indicates 30% higher is acceptable).
            Usage: Helps monitor regressions or improvements in performance.
        */
        "threshold": "130%",

        /* 
            Specifies the comparison method for generating alerts.
            
            Values:
                - "customBiggerIsBetter": Higher values are better; alerts trigger when 
                                        results fall below the threshold.
                - "customSmallerIsBetter": Lower values are better; alerts trigger when 
                                        results exceed the threshold.
        */
        "comparison_method": "customBiggerIsBetter"
    },

    /* 
        Configuration for extracting results from benchmark outputs.
    */
    "result_extraction": {
        /* 
            A regular expression or string to locate the desired result in the benchmark output.
            
            Usage: Helps extract specific performance metrics from raw output logs.
        */
        "search_pattern": "sender",

        /* 
            Indicates which instance of the `search_pattern` in the output should be used as the result.
            
            Format: An integer index (e.g., "7" for the seventh match of `search_pattern`).
        */
        "result_index": 7
    },

    /* 
        Configuration for chart display.
    */
    "chart": {
        /* 
            Specifies the title of the benchmark result. 
            This title is displayed in dashboards and charts.
            
            Format: A concise and descriptive string.
        */
        "title": "[Network] iperf3 sender performance using TCP",

        /* 
            A brief explanation of what the benchmark measures or how it was run.
            
            Usage: Useful for documentation and understanding the benchmark context, 
                which can be displayed as the sub-title in charts.
        */
        "description": "iperf3 -s -B 10.0.2.15",

        /* 
            Defines the unit of measurement for the benchmark result.
            
            Values: Common units like "Mbits/sec", "ns", "ms", etc.
        */
        "unit": "Mbits/sec",

        /* 
            Provides a detailed explanation of what the result represents, with placeholders for dynamic content.
            
            Usage: Used in charts to clarify what each data point signifies.
            
            Dynamic Placeholder:
                - {system}: Automatically replaced by the target system (e.g., Linux, Asterinas) 
                            while processing the result.
        */
        "legend": "Average TCP Bandwidth over virtio-net between Host Linux and Guest {system}"
    },

    /* 
        (Optional) Runtime configuration.
    */
    "runtime_config": {
        /* 
            Defines which specific scheme or configuration of Asterinas is being tested. 
            This aligns with the `SCHEME` parameter in the `asterinas/Makefile`.
            
            Usage:
                - Useful for benchmarking different builds of Asterinas that enable or disable 
                specific features.
                - Allows fine-grained control over performance comparisons between different 
                feature sets.
            
            Possible Values:
                - "iommu": Enables IOMMU support in the build.
                - Other values may correspond to specific configurations defined in the `Makefile`.
            
            Default: If omitted, benchmarks will use the default scheme specified in the `asterinas/Makefile`.
        */
        "aster_scheme": "iommu"
    }
}
```

By following this structure, you can accurately configure and document your benchmark results, ensuring clarity and consistency across your CI system and reporting tools.
