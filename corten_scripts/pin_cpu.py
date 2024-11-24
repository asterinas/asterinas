#!/usr/bin/env python3

import qmp
import sys

def onlinecpu():
    import numa

    o_cpus = []
    for node in range(0,numa.get_max_node()+1):
        for cpu in sorted(numa.node_to_cpus(node)):
            o_cpus.append(cpu)

    return o_cpus


def pin_proc(pid, core):
    import psutil

    try:
        psutil.Process(pid).cpu_affinity([core])
    except ValueError as e:
        print >> sys.stderr, e
        sys.exit(1)


def main():
    # Arguments:
    # 1 --> port number
    # 2 --> number of vcpus

    if len(sys.argv) != 3:
        print("Usage: %s <port> <nr_vcpus>" % sys.argv[0])
        sys.exit(1)

    port = sys.argv[1]
    nr_vcpus = int(sys.argv[2])

    query = qmp.QMPQuery("localhost:%s" % port)

    # Pin CPUs
    print(query.cmd("query-cpus-fast"))
    response = query.cmd("query-cpus-fast")['return']
    o_cpus = [x for x in range(nr_vcpus)]

    for i in range(nr_vcpus):
        pin_proc(int(response[i]['thread-id']), o_cpus[i])

if __name__ == "__main__":
    main()