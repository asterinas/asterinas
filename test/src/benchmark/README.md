# Asterinas Benchmark Collection

The Asterinas Benchmark Collection evaluates the performance of Asterinas in comparison to Linux across a range of benchmarking tools (e.g., LMbench, Sysbench, iPerf) and real-world applications (e.g., Nginx, Redis, SQLite, Memcached). These benchmarks are conducted under various configurations, such as within a single virtual machine (VM) or between a VM and its host.

The benchmarks are run automatically on a nightly basis through continuous integration (CI) pipelines. Results, presented in clear and visually appealing figures and tables, are available for all tier-1 supported platforms.
1. [x86-64](https://asterinas.github.io/benchmark/x86-64/)
2. [Intel TDX](https://asterinas.github.io/benchmark/tdx/)

## File Organization

### Benchmark Suites

The benchmark collection is organized into benchmark suites, each dedicated to a specific benchmarking tool or application. These suites focus on comparing the performance of different operating systems using a particular methodology. Currently, there are eight benchmark suites, each located in its own directory:

- [lmbench](https://github.com/asterinas/asterinas/tree/main/test/src/benchmark/lmbench)
- [sysbench](https://github.com/asterinas/asterinas/tree/main/test/src/benchmark/sysbench)
- [fio](https://github.com/asterinas/asterinas/tree/main/test/src/benchmark/fio)
- [iperf](https://github.com/asterinas/asterinas/tree/main/test/src/benchmark/iperf)
- [sqlite](https://github.com/asterinas/asterinas/tree/main/test/src/benchmark/sqlite)
- [redis](https://github.com/asterinas/asterinas/tree/main/test/src/benchmark/redis)
- [nginx](https://github.com/asterinas/asterinas/tree/main/test/src/benchmark/nginx)
- [memcached](https://github.com/asterinas/asterinas/tree/main/test/src/benchmark/memcached)

Each suite has a corresponding web page (e.g., [LMbench results](https://asterinas.github.io/benchmark/x86-64/lmbench/)) that publishes the latest performance data. At the top of each page, a summary table showcases the most recent results, configured using the `summary.json` file in the suite's directory.

### Benchmark Jobs

Each benchmark suite is divided into benchmark jobs, which perform specific benchmarking tasks. Benchmark jobs are organized into subdirectories under their parent suite directory:

```plaintext
<bench_suite>/
├── <bench_job_a>/
└── <bench_job_b>/
```

Benchmark jobs can be executed using the `bench_linux_and_aster.sh` script located in the `test/src/benchmark/` directory:

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

#### Single Result Jobs

For jobs that produce a single result, the directory is structured as follows:

```plaintext
<bench_suite>/
└── <single_result_job>/
    ├── bench_result.yaml
    └── run.sh
```

The `bench_result.yaml` file contains metadata about the result, including the title, measurement unit, and whether higher or lower values indicate better performance.

#### Multi-Result Jobs

For jobs producing multiple results, the directory includes a `bench_results/` folder:

```plaintext
<bench_suite>/
└── <multi_result_job>/
    ├── bench_results/
    │   ├── <job_a>.yaml
    │   └── <job_b>.yaml
    └── run.sh
```

Each YAML file in the `bench_results/` directory describes a specific result's metadata.

## Adding New Benchmark Jobs

To seamlessly integrate new benchmarks into the Asterinas Benchmark Collection, follow the steps below. These instructions are tailored to the directory structure outlined earlier, where benchmarks are organized under specific suites and jobs.

### Step 1: Add the Directory Structure

Each benchmark job should be added under the corresponding suite in the `test/src/benchmark` directory.

#### Directory Structure

```plaintext
<bench_suite>/
└── <job>/
    ├── host.sh # Only for host-guest jobs
    ├── bench_result.yaml  # or bench_results/ directory for multiple results jobs
    └── run.sh
```

### Step 2: Create Necessary Files

In this step, we need to create several files that are essential for running and managing the benchmarks effectively. Below are the detailed instructions for each required file.

#### Running Scripts

Typically, two scripts are required for each benchmark job: `run.sh` and `host.sh` (for host-guest jobs). These scripts are responsible for executing the benchmark within the guest VM and handling host-side operations, respectively.

Below are the contents of each script for a sample `iperf3` benchmark:

`run.sh`:
```bash
#!/bin/bash

echo "Running iperf3 server..."
/benchmark/bin/iperf3 -s -B 10.0.2.15 --one-off
```

`host.sh`:
```bash
#!/bin/bash

echo "Running iperf3 client"
iperf3 -c $GUEST_SERVER_IP_ADDRESS -f m
```

#### Configuration Files

The configuration files provide metadata about the benchmark jobs and results, such as the regression alerts, chart details, and result extraction patterns. Typically, these files are in YAML format. For single-result jobs, a `bench_result.yaml` file is used, while multi-result jobs have individual YAML files under `bench_results/` for each result. Some fields in these files are necessary while some are optional, depending on the benchmark's requirements. For more information, see the [`bench_result.yaml` format](#the-bench_resultyaml-format) section.

Below are the contents of these files for the sample benchmark:

```yaml
# fio/ext2_no_iommu_seq_write_bw/bench_result.yaml
alert:
  threshold: "125%"
  bigger_is_better: true

result_extraction:
  search_pattern: "bw="
  result_index: 2

chart:
  title: "[Ext2] The bandwidth of sequential writes (IOMMU disabled on Asterinas)"
  description: "fio -filename=/ext2/fio-test -size=1G -bs=1M -direct=1"
  unit: "MB/s"
  legend: "Average file write bandwidth on {system}"

runtime_config:
  aster_scheme: "null"
```

```yaml
# sqlite/ext2_benchmarks/bench_results/ext2_deletes_between.yaml
result_extraction:
  search_pattern: "[0-9]+ DELETEs, numeric BETWEEN, indexed...."
  result_index: 8

chart:
  title: "SQLite Ext2 Deletes Between"

# sqlite/ext2_benchmarks/bench_results/ext2_updates_between.yaml
result_extraction:
  search_pattern: "[0-9]+ UPDATES, numeric BETWEEN, indexed...."
  result_index: 8

chart:
...
```

### Step 3: Update Suite's `summary.json`

Asterinas is an increasingly continuous improvement project. Consequently, while some benchmarks have been incorporated into the Benchmark Collection, their optimization is still ongoing. We do not wish to display these benchmarks on the overview charts. Therefore, we define the benchmarks that should be shown in the `summary.json` file. Only the benchmarks in the `summary.json` file can be displayed on the overview charts. Note that the standalone benchmark results are still available in the respective benchmark suite's page.

To include a new benchmark in the suite's summary table, we need to update the `summary.json` file at the root of the suite. Taking `sqlite` for example:

```jsonc
// sqlite/summary.json
{
    "benchmarks": [
        "ext2_deletes_between",
        "ext2_deletes_individual",
        "ext2_refill_replace",
        "ext2_selects_ipk"
    ]
}
```

### Step 4: Update the CI Configuration

Asterinas employs GitHub Actions for continuous integration (CI) to automatically execute benchmark collection every day. To incorporate the new benchmark into the CI pipeline, it is necessary to update `<bench_suite>/<bench_job>` within the `.github/benchmarks.yml` file.

```yaml
strategy:
    matrix:
    benchmarks:
        - redis/ping_inline_100k_conc20_rps
        - sqlite/ext2_benchmarks
        ...
```

### Step 5: Test, Validate and Commit

Before committing the changes, it is essential to test the new benchmark job locally to ensure it runs correctly. This step helps identify any issues or errors that may arise during the benchmark execution. 

Firstly, we can run the benchmark locally to ensure it works as expected. The following command should finally generate the `result_<bench_suite>-<bench_job>.json` under `asterinas/`. 

```bash
cd asterinas/
bash test/src/benchmark/bench_linux_and_aster.sh <bench_suite>/<bench_job>
```

Secondly, we can validate modifications by running the CI pipeline on our own repository. To do this, we need to modify the `runs-on` field from `self-hosted` to `ubuntu-latest` on `.github/benchmarks.yml`. Then, we can manually trigger the CI pipeline on our own repository to ensure the new benchmark is correctly executed. After validation, we can reverse the `runs-on` field back to `self-hosted`.

Finally, if the new benchmark job runs successfully, we can commit the changes and create a pull request to merge the new benchmark into the main branch.

## The `bench_result.yaml` Format

The `bench_result.yaml` file configures how benchmark results are processed and displayed. Below is an example of the file to give you a big-picture understanding:

```yaml
alert:                        # Alert configuration for performance regression
  threshold: "130%"           # Acceptable performance deviation (e.g., 130% = 30% higher)
  bigger_is_better: true      # true: Higher values are better; false: Lower values are better

result_extraction:            # Result extraction configuration
  search_pattern: "sender"    # Regex or string to locate results
  nth_occurrence: 1           # Optional. Which matched occurrence to use (default to 1).
  result_index: 7             # Match index to use

chart:                        # Chart configuration
  title: "[Network] iperf3 sender performance using TCP"           # Title of the chart
  description: "iperf3 -s -B 10.0.2.15"                            # Context or command associated with the benchmark
  unit: "Mbits/sec"                                                # Measurement unit for the results
  legend: "Average TCP Bandwidth over virtio-net between Host Linux and Guest {system}" # Chart legend with dynamic placeholder {system} supported

runtime_config:              # Runtime configuration
  aster_scheme: "null"       # Corresponds to Makefile parameters, IOMMU is enabled by default (SCHEME=iommu)
  smp: 1                     # Number of CPUs to allocate to the VM
  mem: 8G                  # Memory size in GB to allocate to the VM
```

By adhering to this format, we ensure clarity and consistency in benchmarking workflows and reporting systems.
