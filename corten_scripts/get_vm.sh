#!/bin/bash

function catch () {
    pids=($(ps -ef | grep mem_usage | grep -v grep | awk '{print $2}'))
    file="/proc/${pids[0]}/task/${pids[0]}/children"
    while [ -f $file ]; do
        pid=$(cat $file)
        if [[ "$pid" ]]; then
            pid=${pid%?}
            Nvam=$(cat "/proc/$pid/maps" | wc -l)
            VmPTE=$(grep VmPTE "/proc/$pid/status")
            
            if [[ "$VmPTE" ]]; then
                VmPTE=$(echo $VmPTE |awk '{print $2}')
                #size=$(echo "$Nvam 5"|awk '{printf("%0.3f\n",$1/$2)}') 
                echo "$VmPTE $Nvam" >> output.txt
            fi
        fi
    done

}

function run(){
    echo "" > results
    /home/zjy/asterinas/test/build/initramfs/test/scale/mem_usage $threads &
    catch &
    catch 
    wait
}

function getvm (){
    threads=$1
    echo "" > output.txt
    run
    line=$(cat output.txt | wc -l)
    if [[ $line -le 10 ]]; then
        run
    fi
    ret=$(python3 average.py)
    echo $threads $ret >> vma.txt
}

echo "threads PTEsize(KB) VMAsize(KB)" > vma.txt
for(( ii=1; ii<129; ii++))
do
    getvm $ii
done
