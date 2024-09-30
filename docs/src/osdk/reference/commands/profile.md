# cargo osdk profile

## Overview

The profile command is used to collect stack traces when running the target
kernel in QEMU. It attaches to the GDB server, initiated with the run subcommand,
and collects the stack trace periodically. The collected information can be
used to directly generate a flame graph, or be stored for later analysis using
[the original flame graph tool](https://github.com/brendangregg/FlameGraph).

## Options

`--remote <REMOTE>`:

Specify the address of the remote target.
By default this is `.osdk-gdb-socket`

`--samples <SAMPLES>`:

The number of samples to collect (default 200).
It is recommended to go beyond 100 for performance analysis.

`--interval <INTERVAL>`:

The interval between samples in seconds (default 0.1).

`--parse <PATH>`:

Parse a collected JSON profile file into other formats.

`--format <FORMAT>`:

Possible values:
    - `json`:   The parsed stack trace log from GDB in JSON.
    - `folded`: The folded stack trace for flame graph.
    - `flame-graph`: A SVG flame graph.

If the user does not specify the format, it will be inferred from the
output file extension. If the output file does not have an extension,
the default format is flame graph.

`--cpu-mask <CPU_MASK>`:

The mask of the CPU to generate traces for in the output profile data
(default first 128 cores). This mask is presented as an integer.

`--output <PATH>`:

The path to the output profile data file.

If the user does not specify the output path, it will be generated from
the crate name, current time stamp and the format.

## Examples

To profile a remote QEMU GDB server running some workload for flame graph, do:

```bash
cargo osdk profile --remote :1234 \
	--samples 100 --interval 0.01
```

If wanted a detailed analysis, do:

```bash
cargo osdk profile --remote :1234 \
	--samples 100 --interval 0.01 --output trace.json
```

When you get the above detailed analysis, you can also use the JSON file
to generate the folded format for flame graph.

```bash
cargo osdk profile --parse trace.json --output trace.folded
```
