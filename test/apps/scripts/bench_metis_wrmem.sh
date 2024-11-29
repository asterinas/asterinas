#!/bin/sh

# do /benchmark/bin/metis/wrmem -p $P -s $S for
# $P in 1 16 32 48 64 80 96 112
# $S = $P * 40
for P in 1 16 32 48 64 80 96 112
do
    /benchmark/bin/metis/wrmem -p $P -s $((P * 40))
done
