#!/bin/sh

# usage: corten_benchparsec.sh app thread_count [aster_breakdown]

APP=$1
THREAD_COUNT=$2

if [ -z "${APP}" ] || [ -z "${THREAD_COUNT}" ]; then
    echo "Usage: $0 <app> <thread_count> [aster_breakdown]"
    exit 1
fi

DO_ASTER_BREAKDOWN=$3

run_dir=/benchmark/bin
data_dir=/root
data_source_dir=/benchmark/bin/vm_scale_bench_data

echo "Copying input files to ramfs"
mkdir -p "${data_dir}/${APP}/"
cp -r "${data_source_dir}/${APP}" "${data_dir}/"
echo "Copying input files to ramfs done"

echo "***TEST_START***"

echo "Running application: ${APP} with ${THREAD_COUNT} threads"

if [ "$DO_ASTER_BREAKDOWN" == "aster_breakdown" ]; then
    cat /proc/breakdown-counters
fi

case "${APP}" in
    canneal)
        input_file=$(ls "${data_dir}/canneal/"*.nets | head -n 1)
        "${run_dir}/canneal/canneal" "${THREAD_COUNT}" 15000 2000 "${input_file}" 6000
        ;;
    dedup)
        input_file=$(ls "${data_dir}/dedup/"*.iso | head -n 1)
        "${run_dir}/dedup/dedup" -c -p -v -t "${THREAD_COUNT}" -i "${input_file}" -o output.dat.ddp
        ;;
    streamcluster)
        "${run_dir}/streamcluster/streamcluster" 10 20 128 100000 20000 500 none output.txt "${THREAD_COUNT}"
        ;;
    blackscholes)
        input_file=$(ls "${data_dir}/blackscholes/"*.txt | head -n 1)
        "${run_dir}/blackscholes/blackscholes" "${THREAD_COUNT}" "${input_file}" prices.txt
        ;;
    bodytrack)
        input_file=$(ls "${data_dir}/bodytrack/" | head -n 1)
        "${run_dir}/bodytrack/bodytrack" "${data_dir}/bodytrack/${input_file}" 4 261 4000 5 0 "${THREAD_COUNT}"
        ;;
    facesim)
        cd "${data_dir}/facesim/"
        "${run_dir}/facesim/facesim" -timing -threads "${THREAD_COUNT}" -lastframe 100
        cd -
        ;;
    ferret)
        cd ${data_dir}/ferret/
        "${run_dir}/ferret/ferret" corel lsh queries 50 20 "${THREAD_COUNT}" output.txt
        cd -
        ;;
    fluidanimate)
        input_file=$(ls "${data_dir}/fluidanimate/"*.fluid | head -n 1)
        "${run_dir}/fluidanimate/fluidanimate" "${THREAD_COUNT}" 500 "${input_file}" out.fluid
        ;;
    freqmine)
        input_file=$(ls "${data_dir}/freqmine/"*.dat | head -n 1)
        OMP_NUM_THREADS="${THREAD_COUNT}" "${run_dir}/freqmine/freqmine" "${input_file}" 11000
        ;;
    raytrace)
        input_file=$(ls "${data_dir}/raytrace/"*.obj | head -n 1)
        "${run_dir}/raytrace/rtview" "${input_file}" -automove -nthreads "${THREAD_COUNT}" -frames 200 -res 1920 1080
        ;;
    swaptions)
        "${run_dir}/swaptions/swaptions" -ns 128 -sm 1000000 -nt "${THREAD_COUNT}"
        ;;
    vips)
        input_file=$(ls "${data_dir}/vips/"*.v | head -n 1)
        IM_CONCURRENCY="${THREAD_COUNT}" "${run_dir}/vips/vips" im_benchmark "${input_file}" output.v
        ;;
    x264)
        input_file=$(ls "${data_dir}/x264/"*.y4m | head -n 1)
        "${run_dir}/x264/x264" --quiet --qp 20 --partitions b8x8,i4x4 --ref 5 --direct auto --b-pyramid --weightb --mixed-refs --no-fast-pskip --me umh --subme 7 --analyse b8x8,i4x4 --threads "${THREAD_COUNT}" -o eledream.264 "${input_file}"
        ;;
    *)
        echo "Running application: ${APP}"
        # Add other applications here following the same pattern
        ;;
esac

if [ "$DO_ASTER_BREAKDOWN" == "aster_breakdown" ]; then
    cat /proc/breakdown-counters
fi

echo "***TEST_END***"
