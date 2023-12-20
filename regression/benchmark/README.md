# Introduction to benchmarks

## Sysbench
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

## Membench
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

## Iperf
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
