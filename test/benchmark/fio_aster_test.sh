#!/bin/sh

run_fio_test() {
    local mode="$1"
    local rw="$2"
    local size="$3"
    local bs="$4"

    local rwtype
    local name

    if [ "$mode" == "seq" ]; then
        name="seq-"
    elif [ "$mode" == "rnd" ]; then
        name="rnd-"
        rwtype="rand"
    else
        echo "Error: Invalid mode. Please use 'seq' or 'rnd'."
        exit 1
    fi

    if [ "$rw" == "r" ]; then
        name="${name}r-$bs"
        rwtype="${rwtype}read"
    elif [ "$rw" == "w" ]; then
        name="${name}w-$bs"
        rwtype="${rwtype}write"
    else
        echo "Error: Invalid rw. Please use 'r' or 'w'."
        exit 1
    fi
    
    /benchmark/bin/fio \
        --ioengine=sync \
        --size=$size \
        --rw=$rwtype \
        --filename=/dev/vda \
        --name=$name \
        --bs=$bs \
        --direct=1 \
        --numjobs=1 \
        --fsync_on_close=1 \
        --time_based=1 \
        --runtime=60
}

run_fio_test "seq" "w" "10G" "1M"
run_fio_test "rnd" "w" "10G" "4K"
run_fio_test "rnd" "w" "10G" "32K"
run_fio_test "rnd" "w" "10G" "256K"
run_fio_test "seq" "r" "10G" "1M"
run_fio_test "rnd" "r" "10G" "4K"
run_fio_test "rnd" "r" "10G" "32K"
run_fio_test "rnd" "r" "10G" "256K"
